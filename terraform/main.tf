variable "gcp_project" {
  description = "GCP project ID hosting the bot."
  type        = string
  default     = "aoe2-tournaments"
}

variable "gcp_region" {
  description = "Region for Artifact Registry and the Worker Pool."
  type        = string
  default     = "europe-north1"
}

variable "github_repo" {
  description = "owner/repo of the GitHub repository allowed to deploy via WIF."
  type        = string
  default     = "ZetaTwo/aoe2-tournament-bot"
}

variable "ar_repo_id" {
  description = "Artifact Registry repository ID."
  type        = string
  default     = "aoe2-tournament-bot"
}

variable "image_name" {
  description = "Image name inside the Artifact Registry repo."
  type        = string
  default     = "aoe2-tournament-bot"
}

variable "worker_pool_name" {
  description = "Cloud Run Worker Pool name."
  type        = string
  default     = "aoe2-tournament-bot"
}

variable "bot_runtime_sa" {
  description = "Email of the existing service account the Worker Pool runs as."
  type        = string
  default     = "tournament-bot@aoe2-tournaments.iam.gserviceaccount.com"
}

variable "replays_bucket" {
  description = "GCS bucket the bot uploads replay attachments to."
  type        = string
  default     = "aoe2-tournament-replays"
}

variable "config_secret_id" {
  description = "Secret Manager secret ID holding config.toml."
  type        = string
  default     = "aoe2-tournament-bot-config"
}

data "google_project" "this" {}

data "google_service_account" "bot_runtime" {
  account_id = split("@", var.bot_runtime_sa)[0]
}

data "google_storage_bucket" "replays" {
  name = var.replays_bucket
}

locals {
  ar_image_base = "${var.gcp_region}-docker.pkg.dev/${var.gcp_project}/${var.ar_repo_id}/${var.image_name}"

  wif_pool_id     = "github"
  wif_provider_id = "github"

  # The principal-set that GitHub OIDC tokens from this repo land in.
  wif_principal_set = "principalSet://iam.googleapis.com/projects/${data.google_project.this.number}/locations/global/workloadIdentityPools/${local.wif_pool_id}/attribute.repository/${var.github_repo}"
}
