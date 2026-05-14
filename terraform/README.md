# Terraform: aoe2-tournament-bot infra

Manages the GCP infrastructure the bot needs to be deployed to Cloud Run
Worker Pools from GitHub Actions:

- Workload Identity Federation pool + provider for GitHub OIDC
- A dedicated `github-deployer` service account + IAM bindings
- The `aoe2-tournament-bot` Artifact Registry repo
- A Secret Manager secret holding `config.toml`
- The Cloud Run Worker Pool itself

It does **not** manage:

- The real secret payload (`config.toml` contents) — added out-of-band as a
  new version so it never touches Terraform state. Terraform does create a
  placeholder v1 to satisfy Cloud Run's "the referenced secret version must
  exist" check at Worker-Pool create-time; the real config lands as v2 and
  every subsequent revision resolves `latest` to whatever the newest version
  is at the time the revision is created.
- The existing GCS replay bucket or the Google Sheet — referenced via `data`
  blocks only.
- The image deployed to the Worker Pool — CI rolls forward the image per
  commit; this module ignores `template.containers[0].image` drift on purpose.

## Bootstrap (run once)

### 1. Create the Terraform state bucket

```sh
gcloud storage buckets create gs://aoe2-tournaments-tf-state \
    --project=aoe2-tournaments --location=europe-north1
gcloud storage buckets update gs://aoe2-tournaments-tf-state --versioning
```

(The bucket itself is intentionally not managed by this module — chicken
and egg.)

### 2. Initialise and import existing infra

```sh
cd terraform
terraform init

# AR repo already exists; pull it under TF management.
terraform import google_artifact_registry_repository.bot \
    projects/aoe2-tournaments/locations/europe-north1/repositories/aoe2-tournament-bot
```

The bot's *runtime* service account
(`tournament-bot@aoe2-tournaments.iam.gserviceaccount.com`) is referenced
via a `data` block, **not** imported — we don't want to fight with whatever
IAM bindings it already has on the Sheets API and the replay bucket.

### 3. Plan and apply

```sh
terraform plan
terraform apply
```

Expect to see new resources for: the workload identity pool + provider, the
`github-deployer` SA, three IAM bindings on that SA (WIF impersonation,
`run.developer`, `iam.serviceAccountUser` on the runtime SA), the
`aoe2-tournament-bot-config` secret **and a placeholder v1** of that secret,
an IAM binding granting the runtime SA read access, and the Worker Pool. The
AR repo should show no changes after the import.

The Worker Pool's first revision will boot unhealthy — it's mounting the
placeholder config and pulling whatever's tagged `:latest` in Artifact
Registry. That's expected; the next two steps fix both.

### 4. Populate the config secret (real v2)

```sh
gcloud secrets versions add aoe2-tournament-bot-config \
    --project=aoe2-tournaments --data-file=../config.toml
```

This creates v2 with the real config; the placeholder v1 stays where it is.
The Worker Pool mounts `latest`, which resolves to the newest version each
time a revision is created. So **the existing revision still sees v1** —
the new version doesn't take effect until a new revision is rolled. CI does
this on the next push to `main`; manually, run:

```sh
gcloud run worker-pools update aoe2-tournament-bot \
    --region=europe-north1 --project=aoe2-tournaments
```

To rotate the token or change tournament config later: `gcloud secrets
versions add ...` then trigger a new revision (manually or by pushing).

### 5. Hand the WIF outputs to GitHub

```sh
gh variable set WIF_PROVIDER --body "$(terraform output -raw wif_provider)"
gh variable set DEPLOYER_SA  --body "$(terraform output -raw deployer_sa)"
```

These are stored as **repo variables** (not secrets) — neither value is
sensitive on its own, and `vars.*` is visible in the Actions UI for
easier debugging.

## Day-to-day

- **Deploy code changes**: push to `main`, the CI workflow handles it.
- **Rotate the Discord token or update tournaments**: see step 4 above.
- **Change infra (e.g. bump worker pool memory)**: edit the `.tf` files,
  `terraform plan`, `terraform apply`.

## Beta provider note

`google_cloud_run_v2_worker_pool` currently lives in `google-beta` (see
[worker_pool.tf](worker_pool.tf)). Once Worker Pools graduates to the
stable provider, drop the `provider = google-beta` line on that resource.
