terraform {
  backend "gcs" {
    bucket = "aoe2-tournaments-tf-state"
    prefix = "aoe2-tournament-bot"
  }
}
