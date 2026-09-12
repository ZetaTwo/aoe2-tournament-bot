# Terraform: aoe2-tournament-bot infra

References a single resource: the `tournament-bot@…` runtime service
account (pre-existing, not created here — just a `data` lookup). The bot
itself runs on k3s (see the sibling `infrastructure` repo, Flux CD + GHCR +
`ansible-vault`-encrypted secrets) and authenticates to the Google Sheets
and Cloud Storage APIs with a downloaded key from this SA — don't delete
it.

Does **not** manage sheet sharing (a Drive ACL action, done out-of-band),
or the GCS replay bucket (`aoe2-tournament-replays`) and its permissions.

## Day-to-day

- **Share a new tournament sheet with the runtime SA**:
  ```sh
  RUNTIME_SA=$(terraform output -raw runtime_sa)
  echo "Share each tournament Google Sheet with: $RUNTIME_SA (Viewer)"
  ```
- **Rotate the runtime SA's key**: generate a new key
  (`gcloud iam service-accounts keys create ...`), vault-encrypt it into
  `infrastructure`'s
  `ansible/roles/aoe2_tournament_bot/files/service-account.json`,
  `make ansible-apply` there, then delete the old key
  (`gcloud iam service-accounts keys list/delete`) once the new one's
  confirmed working.
