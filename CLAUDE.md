# Project context for Claude

Discord bot (Rust, [serenity](https://github.com/serenity-rs/serenity)) that
watches "results" channels in AoE2 tournament servers, parses match
reports, uploads attachments to GCS, and appends a row to a Google Sheet.

## Code layout

Single binary crate `aoe2-tournament-bot`. Modules:

- [src/parse.rs](src/parse.rs) — regex parsing of result messages. Pure,
  unit-tested (`TEST_MESSAGE1/2/3` cover the supported formats).
- [src/entry.rs](src/entry.rs) — `ResultsEntry` struct + `get_row()` that
  must produce the 14-column row in [this exact, fixed order](src/entry.rs#L34-L49)
  (`Vec<String>`) — existing sheet readers depend on it.
- [src/config.rs](src/config.rs) — figment-loaded TOML config. **Splits
  across two files** (see "Configuration" below). `Tournament` is the
  validated form; `RawTournament` is what TOML deserializes into.
  `validate_id()` checks each tournament's explicit `id` field (kebab-case)
  and turns it into the GCS prefix.
- [src/tournament.rs](src/tournament.rs) — `match_tournament(input)`
  picks which tournament an incoming message belongs to. Walks the config
  list in order; first match wins; warns on overlapping non-catch-all
  matches. **Match criteria**: `guild_id` (if set), `category` (if set,
  exact Discord category-name match), `channel_pattern` (regex). A
  trailing `catch_all = true` entry catches everything else; overlap with
  it does NOT trigger the dup-match warning.
- [src/sheets.rs](src/sheets.rs) — google-sheets4 wrapper. `ensure_tabs()`
  creates missing tabs on startup via `batchUpdate(AddSheetRequest)`.
  `append_row(tab, row)` does the per-message write.
- [src/gcs.rs](src/gcs.rs) — `gcloud-storage` wrapper (the Yoshidan crate;
  note this differs from the `google-cloud-storage` name on crates.io).
  Picked because `cloud-storage` 0.11 doesn't support the ADC auth path
  the deployment uses: a downloaded service-account key via
  `GOOGLE_APPLICATION_CREDENTIALS`. Pin uses `jwt-rust-crypto` to keep the
  build free of aws-lc/cmake.
- [src/handler.rs](src/handler.rs) — serenity `EventHandler`. Handles
  `message_create` + `message_update`. Resolves the channel + category,
  matches a tournament, builds a `ResultsEntry`, parses, looks up player
  display names, downloads attachments and uploads to GCS, appends the
  row. Failures are just `error!`-logged; admins are notified by the
  `notify` tracing layer (below), **not** by the handler directly.
- [src/notify.rs](src/notify.rs) — `DiscordErrorLayer`, a
  `tracing-subscriber` layer that forwards log events at/above a
  configured `tracing::Level` (constructor param, currently `Level::ERROR`)
  to every `admin_user_ids` entry as a Discord DM. Two non-obvious
  invariants live here:
  - **Init ordering** ([src/main.rs](src/main.rs)): a `tracing`
    subscriber is global + immutable after `.init()`, and the layer needs
    the bot token (only known post-`Config::load`). So basic tracing is
    installed early with the layer as an inert
    `reload::Layer::new(None::<DiscordErrorLayer>)`, then the real layer
    is swapped in via the reload handle after config loads. **Blind
    spot:** `error!`s between `.init()` and the `reload()` are *not*
    DM'd. The only thing that realistically fails there is `Config::load`
    itself, which `?`-returns to stderr via process exit — acceptable.
    Don't "fix" this by reordering config before tracing init.
  - **Loop-safety** (fragile): `on_event` is sync, so the async DM send
    is `tokio::spawn`ed. It can't feed itself **only** because
    `on_event` skips events whose target `starts_with("serenity")` (the
    REST client's logs during the send) or `== module_path!()` (this
    module's own logs). The spawned task logs its own failures with
    `warn!` under the `notify` target, caught by the `module_path!()`
    guard. Keep the send **and** its failure logging inside `notify` and
    at a filtered level — moving the send to another module, or switching
    its failure logging to `error!`, reintroduces an infinite DM loop.
    Widening the level threshold is **not** automatically loop-safe;
    re-check these target exclusions still cover every failure path.
- [src/main.rs](src/main.rs) — wires it up. `tokio::main`. Reads
  `CONFIG_PATH` (default `./config.toml`) and `TOURNAMENTS_PATH` (default
  `./tournaments.toml`).

## Configuration

Two files, merged via figment at startup. **Don't conflate them.**

- **`tournaments.toml`** — checked into git, **baked into the Docker
  image**. Holds the `[[tournaments]]` list. Editing it requires a
  push to `main` so CI builds a new image. See [tournaments.toml](tournaments.toml)
  for the live routing.
- **`config.toml`** — gitignored. Holds `[bot]` (Discord token, admin
  IDs) and `[gcp]` (bucket, sheet ID). In production this is
  `ansible-vault`-encrypted in the `infrastructure` repo and applied as a
  Kubernetes Secret. See [config.example.toml](config.example.toml).

Rotating a Discord token = update `config.toml` in the `infrastructure`
repo's vault, `make ansible-apply` there. Adding a tournament = edit
`tournaments.toml`, commit, push.

## Sheet columns

Row layout is fixed — don't change without coordinating with existing
sheet readers. Order:

`timestamp, message_link, poster, bracket, p1_id, p1_name, p1_score,
p2_id, p2_name, p2_score, map_draft, civ_draft, replays_link,
message_contents`

`bracket` is the Discord category name (distinguishes brackets *within*
a tournament — that's why category isn't part of the match criteria by
default and is recorded in this column regardless).

## Deployment

- **Runtime**: a single-replica Kubernetes `Deployment` on a self-hosted
  k3s cluster (see the sibling `infrastructure` repo), namespace
  `aoe2-tournament-bot`. Runs with `GOOGLE_APPLICATION_CREDENTIALS`
  pointing at a downloaded key for the existing service account
  `tournament-bot@aoe2-tournaments.iam.gserviceaccount.com`, since bare
  k3s has no equivalent to Cloud Run's keyless attached-SA auth.
- **Scaling**: `replicas: 1`, no HPA. Discord gateway is a single
  persistent WebSocket; autoscaling would fight that.
- **Image source**: GitHub Actions ([.github/workflows/ci.yml](.github/workflows/ci.yml))
  builds + pushes to `ghcr.io/zetatwo/aoe2-tournament-bot` on push to
  `main`, then checks out `infrastructure`, bumps the image tag in
  `k8s/aoe2-tournament-bot/deployment.yaml`, commits, and pushes. Flux CD
  (running in the cluster) picks up that commit and reconciles — no
  `kubectl`/deploy step run by CI itself.
- **Auth (GitHub → registry / infra repo)**: GHCR push uses the default
  `GITHUB_TOKEN`; the commit-back uses a fine-grained PAT
  (`INFRA_REPO_PAT` repo secret, scoped to `Contents: Read and write` on
  `infrastructure` only).
- **Infra-as-code**: this repo's [terraform/](terraform/) now only
  references the runtime SA and the replay bucket (`data` blocks, nothing
  managed). The actual Kubernetes manifests and secret-provisioning
  Ansible role live in the `infrastructure` repo:
  `k8s/aoe2-tournament-bot/` and `ansible/roles/aoe2_tournament_bot/`.

### Mount paths inside the container (important)

- `/app/tournaments.toml` — baked in by the Dockerfile.
- `/etc/aoe2-tournament-bot/config.toml` and
  `/etc/aoe2-tournament-bot/service-account.json` — Kubernetes Secret
  volume mount (`aoe2-tournament-bot-secrets`). Bot finds the config via
  `CONFIG_PATH=/etc/aoe2-tournament-bot/config.toml`.
- The mount path is **deliberately not `/app/`** — a directory-level
  volume mount would shadow the baked-in `tournaments.toml`.

### Secret bootstrapping

`config.toml` and the SA key are `ansible-vault`-encrypted files in
`infrastructure`'s `ansible/roles/aoe2_tournament_bot/files/`. That role's
tasks read them via `lookup('file', ...)` (which transparently decrypts
vault content) straight into a `kubernetes.core.k8s` Secret definition —
nothing is ever written to disk on the node. The role also ensures the
`aoe2-tournament-bot` namespace exists, idempotently, since Flux creating
the same namespace from `k8s/aoe2-tournament-bot/namespace.yaml` isn't
guaranteed to run first.

## CI conventions

- `test` job (`cargo test`) **blocks** the `deploy` job.
- `lint` job (`cargo fmt --check` + `cargo clippy -- -D warnings`) runs
  in parallel and **does NOT block** deploy. Intentional: fmt/clippy
  drift shouldn't keep a fix from shipping.

## Common gotchas

- Tournament-config changes need an **image rebuild** to take effect;
  only `config.toml` changes can be rolled without one (update the vault
  file in `infrastructure`, `make ansible-apply`).
- `tournaments.toml` entries' `name` is the Sheet tab name (created on
  startup if missing); `id` is the explicit, separately-set GCS prefix
  (`id = "sf-2026"` → GCS prefix `sf-2026/`).
- The runtime service account needs **Editor** access on the spreadsheet
  (not just Viewer) for `values_append` + `batchUpdate` to work.
- Touching error→Discord forwarding ([src/notify.rs](src/notify.rs)):
  the loop-safety and reload-init invariants there are load-bearing and
  easy to break silently — read that module's bullet under "Code layout"
  before changing the send path, its failure logging, or the level
  threshold.
