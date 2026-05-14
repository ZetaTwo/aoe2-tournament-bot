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

All configuration lives in a TOML file. The bot reads `./config.toml` by
default, or the path set in `CONFIG_PATH`. Log level is controlled by
`RUST_LOG` (e.g. `info`, `debug,serenity=warn`).

See [config.example.toml](config.example.toml) for the full schema. A
tournament block looks like:

```toml
[[tournaments]]
name = "SF 2026"        # also used as the sheet tab name; "sf-2026/" is
                         # derived as the GCS object-key prefix.
guild_id = 1308197621223002112
channel_pattern = "^sf-.*-results$"
```

Tournaments are matched in order; the first match wins. A trailing entry
with `catch_all = true` (and `guild_id` omitted) catches anything that no
specific tournament claimed.

Sheet tabs referenced by `name` must already exist in the spreadsheet; the
bot verifies them on startup and refuses to start otherwise.

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

cp config.example.toml config.toml   # then edit
GOOGLE_APPLICATION_CREDENTIALS=/path/to/service-account.json \
    cargo run --release
```

## Current deployment

Deployment-as-it-stands today. The Rust port introduces a config file
mount, but the rest of the infra is unchanged. Reconsidering this is
explicitly deferred.

- **GCP project**: `aoe2-tournaments`.
- **Service account**: `tournament-bot@aoe2-tournaments.iam.gserviceaccount.com`,
  attached to the VM. Scopes `cloud-platform` and
  `https://www.googleapis.com/auth/spreadsheets`.
- **Artifact Registry**:
  `europe-north1-docker.pkg.dev/aoe2-tournaments/aoe2-tournament-bot/aoe2-tournament-bot:latest`.
- **Compute**: GCE Container-Optimized OS VM named `aoe2-tournament-bot` in
  `europe-north1-b`. Updated via `gcloud compute instances update-container`.
  Auth to GCS / Sheets uses the attached service account through the GCE
  metadata server (no key file required on the VM).
- **GCS bucket**: `aoe2-tournament-replays` (configured per tournament via
  the derived `{name-kebab}/` prefix).
- **Sheet**: shared with the service account; tabs must be created
  out-of-band before adding a tournament to the config.
- **Discord OAuth invite**:
  https://discord.com/oauth2/authorize?client_id=1308197621223002112
- **Google Drive API**:
  https://console.developers.google.com/apis/api/drive.googleapis.com/overview?project=1086054497785

### Deploy commands

```sh
# build image
make            # produces aoe2-tournament-bot.hash
# push image
make publish
# tell the VM to redeploy
gcloud compute instances update-container aoe2-tournament-bot \
    --zone europe-north1-b \
    --container-image europe-north1-docker.pkg.dev/aoe2-tournaments/aoe2-tournament-bot/aoe2-tournament-bot:latest
# (one-off) re-bind service account / scopes
gcloud compute instances set-service-account aoe2-tournament-bot \
    --scopes=cloud-platform,https://www.googleapis.com/auth/spreadsheets \
    --service-account=tournament-bot@aoe2-tournaments.iam.gserviceaccount.com \
    --zone=europe-north1-b
# local impersonation for testing
gcloud auth application-default login \
    --impersonate-service-account tournament-bot@aoe2-tournaments.iam.gserviceaccount.com
```

### Note on the config file in deployment

The Python version configured everything via env vars set on the VM
container spec. The Rust version expects a `config.toml` mounted at
`/app/config.toml`. The VM container spec needs updating to mount this
file (e.g. via a startup script that pulls it from a GCS bucket, or via
GCE metadata). **This is intentionally not done yet** — the user wants to
reconsider deployment separately.
