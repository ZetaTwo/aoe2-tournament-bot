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
tournament's tab in the configured Google Sheet. If the row write fails,
every user listed in `admin_user_ids` is DM'd.

## Configuration

Configuration is split across two TOML files that are merged at startup:

- [tournaments.toml](tournaments.toml) — tournament-to-channel routing.
  Checked into git and baked into the container image, so changes need a
  push-to-`main` (which CI builds + deploys). Default path
  `./tournaments.toml`, overridable via `TOURNAMENTS_PATH`.
- `config.toml` — Discord token, admin IDs, GCP bucket/sheet ID. Never
  committed; stored in Secret Manager (`aoe2-tournament-bot-config`) in
  production. Default path `./config.toml`, overridable via `CONFIG_PATH`.
  See [config.example.toml](config.example.toml) for the schema.

Log level is controlled by `RUST_LOG` (e.g. `info`, `debug,serenity=warn`).

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

The bot is built and rolled forward by [.github/workflows/ci.yml](.github/workflows/ci.yml)
on every push to `main`. It runs as a Cloud Run Worker Pool
(`aoe2-tournament-bot` in `europe-north1`) under the GCP project
`aoe2-tournaments`.

- **Code path**: push to `main` → `cargo test` job runs → on success, the
  `deploy` job builds the image, pushes it tagged `:<sha>` and `:latest` to
  Artifact Registry, then `gcloud run worker-pools update`s the pool.
- **Auth from GitHub to GCP**: Workload Identity Federation. No JSON keys.
  Repo variables `WIF_PROVIDER` and `DEPLOYER_SA` are output by Terraform
  (see below) and set with `gh variable set`.
- **Config / secrets**: `config.toml` lives in Secret Manager as
  `aoe2-tournament-bot-config` and is mounted at
  `/etc/aoe2-tournament-bot/config.toml` in the Worker Pool (the bot finds
  it via `CONFIG_PATH`). Rotating the Discord token =
  `gcloud secrets versions add ...` followed by
  `gcloud run worker-pools update aoe2-tournament-bot --region=europe-north1`
  to roll the revision. `tournaments.toml` is *not* in the secret — it's
  baked into the image, so a routing change is a `git push` to `main`.
- **Infrastructure-as-code**: everything one-time (WIF, the deployer SA,
  the Artifact Registry repo, the config secret, the Worker Pool itself)
  is described in [terraform/](terraform/). See [terraform/README.md](terraform/README.md)
  for the bootstrap order.

### Bot runtime service account

`tournament-bot@aoe2-tournaments.iam.gserviceaccount.com` (predates this
repo). The Terraform module grants it `roles/secretmanager.secretAccessor`
on the config secret; its existing Sheets API and `aoe2-tournament-replays`
GCS bucket permissions carry over.

### Useful links

- Discord invite: https://discord.com/oauth2/authorize?client_id=1308197621223002112
- Google Drive API: https://console.developers.google.com/apis/api/drive.googleapis.com/overview?project=1086054497785

### Local impersonation for testing

```sh
gcloud auth application-default login \
    --impersonate-service-account tournament-bot@aoe2-tournaments.iam.gserviceaccount.com
cargo run --release
```

### Retiring the old GCE VM

The previous deployment was a Container-Optimized OS GCE VM
(`aoe2-tournament-bot` in `europe-north1-b`) updated via
`gcloud compute instances update-container`. Once the Worker Pool has been
verified end-to-end, retire it manually:

```sh
gcloud compute instances delete aoe2-tournament-bot --zone=europe-north1-b
```
