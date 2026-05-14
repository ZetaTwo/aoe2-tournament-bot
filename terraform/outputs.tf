output "wif_provider" {
  description = "Full WIF provider resource name. Set as the WIF_PROVIDER repo variable in GitHub."
  value       = google_iam_workload_identity_pool_provider.github.name
}

output "deployer_sa" {
  description = "Deployer service account email. Set as the DEPLOYER_SA repo variable in GitHub."
  value       = google_service_account.deployer.email
}

output "worker_pool" {
  description = "Cloud Run Worker Pool resource name."
  value       = google_cloud_run_v2_worker_pool.bot.name
}

output "ar_image_base" {
  description = "Fully-qualified image path the CI workflow pushes to."
  value       = local.ar_image_base
}
