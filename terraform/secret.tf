# Secret container. Real config payload is populated out-of-band so it
# never lands in Terraform state:
#   gcloud secrets versions add aoe2-tournament-bot-config \
#       --project=aoe2-tournaments --data-file=config.toml
resource "google_secret_manager_secret" "config" {
  secret_id = var.config_secret_id

  replication {
    auto {}
  }
}

# Placeholder v1. Cloud Run validates at create-time that the secret version
# referenced by a volume mount exists, so we can't bring the Worker Pool up
# until *some* version exists. This placeholder satisfies that check; the
# real config is added later as v2+, and the Worker Pool's `latest` mount
# resolves to whichever version is newest on each revision creation.
resource "google_secret_manager_secret_version" "bootstrap" {
  secret      = google_secret_manager_secret.config.id
  secret_data = "# placeholder created by Terraform — replace via 'gcloud secrets versions add'\n"

  lifecycle {
    # Don't tempt anyone into editing this; the real config arrives as a
    # later version and this v1 is intentionally left as-is.
    ignore_changes = [secret_data]
  }
}
