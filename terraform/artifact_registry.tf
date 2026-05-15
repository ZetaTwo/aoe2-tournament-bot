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

  cleanup_policy_dry_run = false

  # Keep only the 5 most recent image versions. A KEEP-only policy is a
  # no-op (KEEP just protects artifacts from DELETE policies), so the
  # "delete-all" policy sweeps everything and the KEEP policies exempt what
  # we want to retain — KEEP takes precedence over DELETE. CI rolls a fresh
  # WP revision every push to main, so the live image is always the most
  # recent; 5 leaves roughly a 4-deploy rollback window.
  #
  # The CI buildx registry cache lives in this same repo under the
  # `buildcache` tag (see .github/workflows/ci.yml). "keep-buildcache"
  # protects whatever currently carries that tag so a build can't evict its
  # own cache; stale (now-untagged) older cache digests are NOT protected
  # and get swept by "delete-all", which keeps cache storage bounded.
  cleanup_policies {
    id     = "delete-all"
    action = "DELETE"
    condition {
      tag_state = "ANY"
    }
  }

  cleanup_policies {
    id     = "keep-last-5"
    action = "KEEP"
    most_recent_versions {
      keep_count = 5
    }
  }

  cleanup_policies {
    id     = "keep-buildcache"
    action = "KEEP"
    condition {
      tag_state    = "TAGGED"
      tag_prefixes = ["buildcache"]
    }
  }
}

resource "google_artifact_registry_repository_iam_member" "deployer_writer" {
  project    = google_artifact_registry_repository.bot.project
  location   = google_artifact_registry_repository.bot.location
  repository = google_artifact_registry_repository.bot.name
  role       = "roles/artifactregistry.writer"
  member     = "serviceAccount:${google_service_account.deployer.email}"
}
