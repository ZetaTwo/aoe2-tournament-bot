# Existing repo, imported once with:
#   terraform import google_artifact_registry_repository.bot \
#     projects/aoe2-tournaments/locations/europe-north1/repositories/aoe2-tournament-bot
#
# Only the three fields that must match the live repo are pinned here; any
# other attributes (description, labels, cleanup policies, ...) stay as
# whatever was set out-of-band. If `terraform plan` shows a diff right after
# import, copy the relevant attribute from `gcloud artifacts repositories
# describe ...` output into this resource and re-plan.
resource "google_artifact_registry_repository" "bot" {
  location      = var.gcp_region
  repository_id = var.ar_repo_id
  format        = "DOCKER"

  docker_config {
    immutable_tags = false
  }
  cleanup_policy_dry_run = true
}

resource "google_artifact_registry_repository_iam_member" "deployer_writer" {
  project    = google_artifact_registry_repository.bot.project
  location   = google_artifact_registry_repository.bot.location
  repository = google_artifact_registry_repository.bot.name
  role       = "roles/artifactregistry.writer"
  member     = "serviceAccount:${google_service_account.deployer.email}"
}
