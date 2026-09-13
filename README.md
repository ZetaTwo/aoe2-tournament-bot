# aoe2-tournament-bot

A Discord bot that captures AoE2 tournament match-result messages into a
Google Sheet and uploads replay attachments to a GCS bucket. Written in Rust
using [serenity](https://github.com/serenity-rs/serenity).

## What it does

The bot watches Discord channels whose names match one of the configured
tournament patterns. For each new or edited message it tries to extract:

- the two players mentioned with `@user` tags
- a `Map: <url>` / `Map draft: <url>` link
- a `Civs: <url>` / `Civ draft: <url>` link
- a score line of the form `<digits> <separator> <digits>` (so `3-0`,
  `||0:3||`, `2 - 1`, ...)

It looks up the players' Discord display names, downloads any attached
replay files into the configured GCS bucket, and appends a row to the
tournament's tab in the configured Google Sheet. Failures are logged as
structured JSON to stdout; alerting on them is handled by infra, not the
bot itself.

## Configuration

Configuration is split across two TOML files that are merged at startup:

- [tournaments.toml](tournaments.toml) — tournament-to-channel routing.
  Checked into git and baked into the container image, so changes need a
  push-to-`main` (which CI builds + deploys). Default path
  `./tournaments.toml`, overridable via `TOURNAMENTS_PATH`.
- `config.toml` — Discord token, GCP bucket/sheet ID. Never committed;
  `ansible-vault`-encrypted in the sibling `infrastructure` repo's
  `ansible/roles/aoe2_tournament_bot/files/config.toml` and applied as a
  Kubernetes Secret in production. Default path `./config.toml`,
  overridable via `CONFIG_PATH`. See [config.example.toml](config.example.toml)
  for the schema.

Logs are structured JSON on stdout; level is controlled by `RUST_LOG`
(e.g. `info`, `debug,serenity=warn`). Infra tails and alerts on these logs
— the bot no longer DMs admins on failure.

A tournament block looks like:

```toml
[[tournaments]]
name = "SF 2026"        # also used as the sheet tab name; "sf-2026/" is
                         # derived as the GCS object-key prefix.
guild_id = 1308197621223002112
category = "SF 2026 Bracket"   # optional; if set, message's Discord
                                # category name must equal this exactly.
channel_pattern = "^sf-.*-results$"
```

Tournaments are matched in order; the first match wins. All set
conditions (`guild_id`, `category`, `channel_pattern`) must match. A
trailing entry with `catch_all = true` (and `guild_id` omitted) catches
anything that no specific tournament claimed.

Sheet tabs referenced by `name` are created on startup if they don't exist
yet, so adding a new tournament just means adding a `[[tournaments]]` block
to [tournaments.toml](tournaments.toml) and pushing to `main`.

## Sheet columns

Every row has the same shape, in this order:

`timestamp, message_link, poster, bracket (= Discord category), player1_id,
player1_name, player1_score, player2_id, player2_name, player2_score,
map_draft, civ_draft, replays_link, message_contents`

The `bracket` column carries the Discord category name and distinguishes
brackets *within* a tournament (e.g. "Recruit SF" vs "General SF" both
inside the SF tournament's tab).

## Local development

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --release

cp config.example.toml config.toml   # then edit (token, sheet ID, etc.)
# tournaments.toml is already in the repo — edit it directly if you want
# to test routing changes locally.
GOOGLE_APPLICATION_CREDENTIALS=/path/to/service-account.json \
    cargo run --release
```

## Deployment

The bot runs as a single-replica Kubernetes `Deployment` on a self-hosted
k3s cluster, managed via GitOps in the sibling `infrastructure` repo (Flux
CD watches that repo's `k8s/` tree and reconciles it). No Service or
Ingress — it's not a web app, just a background worker holding a
persistent Discord gateway connection (`replicas: 1`, never autoscaled).

- **Code path**: push to `main` → `cargo test` job runs → on success, the
  `deploy` job builds the image, pushes it to
  `ghcr.io/zetatwo/aoe2-tournament-bot` tagged `:<sha>`, then bumps that
  tag in `infrastructure`'s `k8s/aoe2-tournament-bot/deployment.yaml` and
  pushes — Flux reconciles the new image within about a minute.
- **Auth from GitHub to registry / infra repo**: GHCR push uses the
  default `GITHUB_TOKEN`; the commit-back to `infrastructure` uses a
  fine-grained PAT (`INFRA_REPO_PAT` repo secret, scoped to
  `Contents: Read and write` on that one repo).
- **Config / secrets**: `config.toml` is `ansible-vault`-encrypted at
  `infrastructure`'s `ansible/roles/aoe2_tournament_bot/files/config.toml`
  and applied as a Kubernetes `Secret`, mounted at
  `/etc/aoe2-tournament-bot/config.toml` (the bot finds it via
  `CONFIG_PATH`). Rotating the Discord token means updating that file and
  running `make ansible-apply` in `infrastructure`. `tournaments.toml` is
  *not* a secret — it's baked into the image, so a routing change is still
  just a `git push` to `main`.
- **Infrastructure-as-code**: the Kubernetes manifests live in
  `infrastructure`'s `k8s/aoe2-tournament-bot/`; the Ansible role
  provisioning the namespace + secret is
  `ansible/roles/aoe2_tournament_bot/`. This repo's own
  [terraform/](terraform/) only keeps a reference to the runtime service
  account and the GCS replay bucket — see [terraform/README.md](terraform/README.md).

### Bot runtime service account

`tournament-bot@aoe2-tournaments.iam.gserviceaccount.com`. The pod
authenticates to the Sheets and GCS APIs with a downloaded key from this
SA (`GOOGLE_APPLICATION_CREDENTIALS`), mounted the same way as
`config.toml`. Its Sheets API and `aoe2-tournament-replays` GCS bucket
permissions are unchanged.

### Useful links

- Discord invite: https://discord.com/oauth2/authorize?client_id=1308197621223002112
- Google Drive API: https://console.developers.google.com/apis/api/drive.googleapis.com/overview?project=1086054497785

### Local impersonation for testing

```sh
gcloud auth application-default login \
    --impersonate-service-account tournament-bot@aoe2-tournaments.iam.gserviceaccount.com
cargo run --release
```
