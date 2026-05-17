# Project context for Claude

Discord bot (Rust, [serenity](https://github.com/serenity-rs/serenity)) that
watches "results" channels in AoE2 tournament servers, parses match
reports, uploads attachments to GCS, and appends a row to a Google Sheet.
Ports from a Python implementation (now deleted; see the `Started Rust
port` commit for the cut-over).

## Code layout

Single binary crate `aoe2-tournament-bot`. Modules:

- [src/parse.rs](src/parse.rs) — regex parsing of result messages. Pure,
  unit-tested. The three Python tests are ported verbatim (`TEST_MESSAGE1/2/3`).
- [src/entry.rs](src/entry.rs) — `ResultsEntry` struct + `get_row()` that
  must produce the 14-column row in [the exact order the Python bot used](src/entry.rs#L34-L49)
  (`Vec<String>`).
- [src/config.rs](src/config.rs) — figment-loaded TOML config. **Splits
  across two files** (see "Configuration" below). `Tournament` is the
  validated form; `RawTournament` is what TOML deserializes into.
  `kebab_case_prefix()` derives GCS prefixes from tournament names.
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
- [src/gcs.rs](src/gcs.rs) — `gcloud-storage` wrapper (the Yoshidan crate,
  picked because `cloud-storage` 0.11 doesn't support the GCE metadata
  server auth path the deployment uses). It was originally the
  `google-cloud-storage` crate; that name was **donated to Google's
  official SDK**, so the Yoshidan crate continues under the name
  `gcloud-storage` (same API). Pin uses `jwt-rust-crypto` to keep the
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
  IDs) and `[gcp]` (bucket, sheet ID). In production this lives in
  Secret Manager as `aoe2-tournament-bot-config`. See
  [config.example.toml](config.example.toml).

Rotating a Discord token = `gcloud secrets versions add ...` then roll a
new Worker Pool revision. Adding a tournament = edit `tournaments.toml`,
commit, push.

## Sheet columns

Row layout matches the Python bot's (don't change without coordinating with
existing sheet readers). Order:

`timestamp, message_link, poster, bracket, p1_id, p1_name, p1_score,
p2_id, p2_name, p2_score, map_draft, civ_draft, replays_link,
message_contents`

`bracket` is the Discord category name (distinguishes brackets *within*
a tournament — that's why category isn't part of the match criteria by
default and is recorded in this column regardless).

## Deployment

- **Runtime**: Cloud Run Worker Pool `aoe2-tournament-bot` in
  `europe-north1`, GCP project `aoe2-tournaments`, runs as the existing
  service account `tournament-bot@aoe2-tournaments.iam.gserviceaccount.com`.
- **Scaling**: `MANUAL` with `manual_instance_count = 1`. Discord
  gateway is a single persistent WebSocket; autoscale would idle this to
  zero.
- **Image source**: GitHub Actions ([.github/workflows/ci.yml](.github/workflows/ci.yml))
  builds + pushes to Artifact Registry on push to `main`, then runs
  `gcloud run worker-pools update --image=...` to roll a revision.
- **Auth (GitHub → GCP)**: Workload Identity Federation. Two repo
  *variables* (not secrets): `WIF_PROVIDER`, `DEPLOYER_SA`. Output by
  Terraform.
- **Infra-as-code**: [terraform/](terraform/). One-time `terraform
  apply` brings up everything except: the TF state bucket itself
  (chicken-and-egg), the real secret payload (out-of-band), and the
  image (CI owns it — `ignore_changes` on `template[0].containers[0].image`).

### Mount paths inside the container (important)

- `/app/tournaments.toml` — baked in by the Dockerfile.
- `/etc/aoe2-tournament-bot/config.toml` — secret volume mount. Bot finds
  it via `CONFIG_PATH=/etc/aoe2-tournament-bot/config.toml` (env set on
  the Worker Pool container).
- The mount path is **deliberately not `/app/`** — a directory-level
  volume mount would shadow the baked-in `tournaments.toml`.

### Secret bootstrapping

Cloud Run validates at create time that the secret version referenced
by a volume mount exists. Terraform therefore creates a *placeholder* v1
of `aoe2-tournament-bot-config` (see [terraform/secret.tf](terraform/secret.tf))
with `lifecycle.ignore_changes = [secret_data]`. The real config is
added as v2+ out-of-band; `latest` in the WP volume mount resolves to
whatever's newest at revision-creation time.

## CI conventions

- `test` job (`cargo test`) **blocks** the `deploy` job.
- `lint` job (`cargo fmt --check` + `cargo clippy -- -D warnings`) runs
  in parallel and **does NOT block** deploy. Intentional: fmt/clippy
  drift shouldn't keep a fix from shipping.

## Common gotchas

- `google_cloud_run_v2_worker_pool` defaults `deletion_protection = true`;
  the resource explicitly sets it to `false` so plan-driven replacements
  work without manual intervention.
- Tournament-config changes need an **image rebuild** to take effect; only
  config-secret changes can be rolled with `gcloud secrets versions add`
  + WP revision.
- `tournaments.toml` entries' `name` doubles as the Sheet tab name (created
  on startup if missing) and as the kebab-cased GCS prefix
  (`name = "SF 2026"` → tab `SF 2026`, GCS prefix `sf-2026/`).
- The runtime service account needs **Editor** access on the spreadsheet
  (not just Viewer) for `values_append` + `batchUpdate` to work.
- Touching error→Discord forwarding ([src/notify.rs](src/notify.rs)):
  the loop-safety and reload-init invariants there are load-bearing and
  easy to break silently — read that module's bullet under "Code layout"
  before changing the send path, its failure logging, or the level
  threshold.

## Migration state

The Rust port is on `main`. Live infrastructure is mid-migration from
the previous GCE COS VM (`aoe2-tournament-bot` in `europe-north1-b`) to
the Cloud Run Worker Pool. [MIGRATION.md](MIGRATION.md) tracks the
remaining cutover steps; delete that file once the GCE VM is gone.
