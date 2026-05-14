# Migration: GCE VM → Cloud Run Worker Pool

One-time checklist for moving the bot from the Python build running on the
`aoe2-tournament-bot` Compute Engine VM to the Rust build running as a
Cloud Run Worker Pool. Delete this file once the migration is done.

## Before you start

You'll need:

- `gcloud` authenticated as a project owner / IAM admin on `aoe2-tournaments`.
- `terraform` ≥ 1.6 and `gh` (GitHub CLI) on your laptop.
- The current `DISCORD_TOKEN`, `SHEET_ID`, and `ADMIN_USER_ID` values from
  the running VM's container env (`gcloud compute instances describe
  aoe2-tournament-bot --zone=europe-north1-b` will print them).
- A short maintenance window. The cutover step stops the GCE VM and then
  brings the Worker Pool up — expect a few minutes where the bot does not
  ingest new messages. Discord edits to messages posted during the window
  are still picked up afterwards because the Rust bot processes
  `message_update`.

## 0. Sanity-check the existing infrastructure

These commands shouldn't change anything. They just confirm the things the
new setup expects to already exist:

```sh
# Service account still around with the right email
gcloud iam service-accounts describe \
    tournament-bot@aoe2-tournaments.iam.gserviceaccount.com

# Replay bucket still around
gcloud storage buckets describe gs://aoe2-tournament-replays

# Artifact Registry repo still around (will be imported into Terraform)
gcloud artifacts repositories describe aoe2-tournament-bot \
    --location=europe-north1
```

## 1. Prepare the Google Sheet

We're going with **"leave historical rows where they are"**: don't touch
the existing rows.

1. Open the spreadsheet (its ID is in `SHEET_ID` on the VM env).
2. Rename the current/default tab to something like `Archive (pre-Rust)`
   so it's clear which rows came from the Python build.
3. Confirm the sheet is shared with
   `tournament-bot@aoe2-tournaments.iam.gserviceaccount.com` (sheet-level
   sharing covers all tabs — verify under `Share`).

You do **not** need to pre-create tab(s) for each tournament. On startup
the bot inspects the spreadsheet and adds any missing tabs whose names
match a configured `[[tournaments]] name` (see
[src/sheets.rs:77-112](src/sheets.rs#L77-L112)).

## 2. Edit `tournaments.toml` and write `config.toml`

Configuration is split across two files:

**`tournaments.toml`** — checked into git, baked into the image. Edit and
commit (no need to wait until later — pushing to `main` after step 4 will
roll a build that includes it):

- Add a `[[tournaments]]` block per Discord category/channel-pattern you
  want to capture.
- Keep the trailing `catch_all = true` block as a backstop.

**`config.toml`** — local-only, ends up in Secret Manager. Copy
`config.example.toml` to `config.toml` and fill in:

- `bot.discord_token` — same value the GCE VM uses today.
- `bot.admin_user_ids` — at least the existing `ADMIN_USER_ID`; can be a
  list.
- `gcp.bucket` — `aoe2-tournament-replays`.
- `gcp.sheet_id` — same `SHEET_ID` as today.

Do **not** commit `config.toml` (`.gitignore` already excludes it).

## 3. Bootstrap Terraform-managed infra

```sh
# State bucket (one-time)
gcloud storage buckets create gs://aoe2-tournaments-tf-state \
    --project=aoe2-tournaments --location=europe-north1
gcloud storage buckets update gs://aoe2-tournaments-tf-state --versioning

# Init + import the existing AR repo
terraform -chdir=terraform init
terraform -chdir=terraform import google_artifact_registry_repository.bot \
    projects/aoe2-tournaments/locations/europe-north1/repositories/aoe2-tournament-bot

# Review and apply
terraform -chdir=terraform plan
terraform -chdir=terraform apply
```

After the `import`, `terraform plan` should show **no diff** on the
`google_artifact_registry_repository.bot` resource — only the
genuinely new resources should appear in the plan. If the AR repo does
show a diff, that means a field is set on the live repo but missing
from [terraform/artifact_registry.tf](terraform/artifact_registry.tf)
(or vice versa). Use `gcloud artifacts repositories describe
aoe2-tournament-bot --location=europe-north1` to see the live values
and either:

- add the field to the resource block so Terraform stops trying to
  change it, or
- accept the diff if you actually want Terraform to overwrite that
  field on the next apply.

The apply itself creates: the workload identity pool + provider, the
`github-deployer` SA with its three role bindings, the
`aoe2-tournament-bot-config` secret with a **placeholder v1** (Cloud Run
won't create the Worker Pool unless *some* version of the mounted secret
already exists — the placeholder satisfies that), the runtime SA's
secret-reader binding, and the Worker Pool itself.

The Worker Pool's first revision will be unhealthy: it's pulling whatever
`:latest` in Artifact Registry currently is (the last Python image) and
mounting a placeholder config. That's expected. Both get replaced in the
next two steps.

## 4. Populate the config secret and GitHub variables

```sh
# Push the config you wrote in step 2 into Secret Manager as v2.
# (Terraform created a placeholder v1; you don't need to touch it.)
gcloud secrets versions add aoe2-tournament-bot-config \
    --project=aoe2-tournaments --data-file=config.toml

# Hand the Terraform outputs to GitHub Actions
gh variable set WIF_PROVIDER --body "$(terraform -chdir=terraform output -raw wif_provider)"
gh variable set DEPLOYER_SA  --body "$(terraform -chdir=terraform output -raw deployer_sa)"
```

## 5. Cutover

Merge the Rust port to `main` (or push a commit, depending on your branch
strategy). The CI workflow runs `cargo test` → builds & pushes the image
tagged with the commit SHA → `gcloud run worker-pools update`s the pool to
that SHA.

Then **before** the new pool is actually serving (i.e. while the deploy
job is still running), stop the GCE VM to avoid double-writes:

```sh
gcloud compute instances stop aoe2-tournament-bot --zone=europe-north1-b
```

Two bots reading the same channels would post duplicate rows to the sheet
(harmless but messy) and double-upload attachments.

Watch the GitHub Actions run finish, then tail the Worker Pool logs:

```sh
gcloud run worker-pools logs read aoe2-tournament-bot \
    --region=europe-north1 --limit=50
```

You're looking for the `Logged in as <bot name>` line and no errors about
missing sheet tabs or secret mounts.

## 6. End-to-end verification

Post a test results message in one of the configured tournament channels.
Expect:

- A new row in the matched tournament's tab (not the archive tab).
- An object in `gs://aoe2-tournament-replays/<tournament-prefix>/...` for
  each attachment.
- No DM from the bot complaining about a failed sheet write.

Edit the same message; verify the bot adds another row reflecting the
edit (the bot re-processes `message_update` events).

## 7. Decommission the GCE VM

After a confidence period (a day or two of successful operation):

```sh
gcloud compute instances delete aoe2-tournament-bot --zone=europe-north1-b
```

There is no other GCE-VM-specific resource to clean up; the firewall
rules, networks, and IAM bindings the VM used either belong to other
things or are removed automatically with the instance.

## Rollback (only if step 6 reveals something broken)

1. `gcloud compute instances start aoe2-tournament-bot --zone=europe-north1-b`
   to bring the Python bot back up.
2. Either pause the Worker Pool revisions
   (`gcloud run worker-pools update aoe2-tournament-bot
   --region=europe-north1 --min-instances=0 --max-instances=0`)
   or delete the pool (`gcloud run worker-pools delete ...`) so it stops
   competing with the VM.
3. File an issue describing what broke. The Terraform state and the
   secret can stay where they are; nothing about them is harmful to leave
   in place while the Python bot is back in charge.

Once the issue is fixed, restart the migration from step 5.

## What this migration does **not** change

- The Discord bot's identity/client_id. Same token, same OAuth invite link,
  same Discord application.
- The `aoe2-tournament-replays` GCS bucket itself (only the object-key
  layout for **new** uploads — they now sit under per-tournament
  prefixes; old objects at the bucket root are untouched).
- The Google Sheet ID. Only the tab structure changes (per step 1).
- The runtime service account
  (`tournament-bot@aoe2-tournaments.iam.gserviceaccount.com`) — same
  identity, with one new IAM binding for the config secret.
