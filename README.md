https://discord.com/oauth2/authorize?client_id=1308197621223002112

https://console.developers.google.com/apis/api/drive.googleapis.com/overview?project=1086054497785


GCP:
- Service account
- Artifact registry + (assign permissions)
- Upload image
- COS GCE machine: image + env vars + API scope 
- GCS bucket + (assign permissions)

Drive:
- Create Sheet and share with service account

gcloud auth application-default login --impersonate-service-account tournament-bot@aoe2-tournaments.iam.gserviceaccount.com
gcloud compute instances update-container aoe2-tournament-bot --zone europe-north1-b --container-image europe-north1-docker.pkg.dev/aoe2-tournaments/aoe2-tournament-bot/aoe2-tournament-bot:latest


"Message link",
"Poster Display Name",
"Bracket",
"Player 1 ID",
"Player 1 Display Name",
"Player 1 Score",
"Player 2 Display Name",
"Player 2 ID",
"Player 2 Score",
"Map Draft",
"Civ Draft",
"Replay links",
"Message contents",