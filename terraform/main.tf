variable "gcp_project" {
  description = "GCP project ID hosting the bot's remaining resources."
  type        = string
  default     = "aoe2-tournaments"
}

variable "gcp_region" {
  description = "Region for GCP resources."
  type        = string
  default     = "europe-north1"
}

variable "bot_runtime_sa" {
  description = "Email of the existing service account the bot runs as."
  type        = string
  default     = "tournament-bot@aoe2-tournaments.iam.gserviceaccount.com"
}

variable "replays_bucket" {
  description = "GCS bucket the bot uploads replay attachments to."
  type        = string
  default     = "aoe2-tournament-replays"
}

data "google_service_account" "bot_runtime" {
  account_id = split("@", var.bot_runtime_sa)[0]
}

# Not managed or referenced elsewhere in this config — a data-only reminder
# that the bot depends on this bucket existing. Its IAM grants are set up
# outside Terraform (gcloud/console), not tracked here.
data "google_storage_bucket" "replays" {
  name = var.replays_bucket
}
