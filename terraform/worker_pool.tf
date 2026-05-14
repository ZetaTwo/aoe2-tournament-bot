# Worker Pool currently lives in the google-beta provider.
# Switch to `provider = google` once it graduates.
resource "google_cloud_run_v2_worker_pool" "bot" {
  provider = google-beta

  name     = var.worker_pool_name
  location = var.gcp_region

  # Resources we expect to evolve via plan/apply rather than be irreplaceable
  # production artefacts; the provider defaults to `true`, which blocks any
  # destroy/replace until you flip this to false and apply.
  deletion_protection = false

  # Discord gateway is a single persistent WebSocket — we want exactly one
  # instance running at all times, not autoscaled based on CPU.
  scaling {
    manual_instance_count = 1
  }

  template {
    service_account = data.google_service_account.bot_runtime.email

    containers {
      name = "aoe2-tournament-bot-1"

      # Placeholder. CI replaces this per-deploy via `gcloud run worker-pools
      # update --image=...`, which is why `ignore_changes` below masks it.
      image = "${local.ar_image_base}:latest"

      resources {
        limits = {
          cpu    = "1"
          memory = "512Mi"
        }
      }


      env {
        name  = "RUST_LOG"
        value = "info"
      }

      volume_mounts {
        name       = "config"
        mount_path = "/app"
      }
    }

    volumes {
      name = "config"
      secret {
        secret = google_secret_manager_secret.config.secret_id
        items {
          version = "latest"
          path    = "config.toml"
        }
      }
    }
  }

  lifecycle {
    # CI owns the image. Terraform owns everything else about the worker pool.
    ignore_changes = [
      template[0].containers[0].image,
    ]
  }

  depends_on = [
    google_secret_manager_secret_iam_member.runtime_config_reader,
    google_secret_manager_secret_version.bootstrap,
  ]
}
