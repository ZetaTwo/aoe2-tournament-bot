//! Background queue that performs the GCS upload(s) and Sheets append for a
//! matched message, off the Discord gateway task.
//!
//! `SheetsClient`/`GcsClient` already retry individual HTTP calls briefly
//! (a few seconds, see [`crate::retry::backoff`]). This module adds a much
//! longer retry horizon ([`crate::retry::job_backoff`]) on top, scoped
//! narrowly per step (one attachment upload, or the sheet append) rather
//! than around the whole job — retrying the append never needs to touch
//! GCS again, and retrying an attachment upload never re-touches one that
//! already succeeded, since each such step is only retried until it first
//! succeeds. If a step's extended retry is exhausted, the job is dropped
//! and logged at `error!`; the message is not retried again.

use std::sync::Arc;

use anyhow::{Context as _, Result};
use backon::Retryable;
use chrono::{SecondsFormat, Utc};
use serenity::all::{Attachment, Http, MessageId, UserId};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::{
    entry::ResultsEntry, gcs::GcsClient, parse::parse_message_content, retry, sheets::SheetsClient,
};

/// Which gateway event delivered the message. On [`MessageEvent::Updated`],
/// Discord guarantees the attachments are unchanged from the original post,
/// so the files are already in GCS and must not be re-uploaded — an overwrite
/// needs `storage.objects.delete`, which the runtime service account lacks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageEvent {
    Created,
    Updated,
}

/// Everything the worker needs to process one matched message, extracted up
/// front in `Handler::process_message` so the job depends on neither
/// `Config`/`Tournament` nor the full serenity `Context`.
pub struct Job {
    pub http: Arc<Http>,
    pub message_id: MessageId,
    pub jump_url: String,
    pub poster: String,
    pub content: String,
    pub category: Option<String>,
    pub gcs_prefix: String,
    pub sheet_tab: String,
    pub attachments: Vec<Attachment>,
    pub event: MessageEvent,
}

pub async fn run(
    mut jobs: mpsc::UnboundedReceiver<Job>,
    sheets: Arc<SheetsClient>,
    gcs: Arc<GcsClient>,
) {
    while let Some(job) = jobs.recv().await {
        run_job(&sheets, &gcs, job).await;
    }
    info!("job queue closed; background worker exiting");
}

async fn run_job(sheets: &SheetsClient, gcs: &GcsClient, job: Job) {
    let message_id = job.message_id;
    let entry = match build_entry(gcs, &job).await {
        Ok(entry) => entry,
        Err(e) => {
            error!("building results entry failed for message {message_id} after extended retry: {e:#}");
            return;
        }
    };

    let now = Utc::now();
    let mut row = Vec::with_capacity(crate::entry::SHEET_COLUMN_COUNT);
    row.push(now.to_rfc3339_opts(SecondsFormat::Secs, false));
    row.extend(entry.get_row());

    let result = (|| async { sheets.append_row(&job.sheet_tab, row.clone()).await })
        .retry(retry::job_backoff())
        .when(|_| true)
        .notify(retry::log_retry("worker.append_row"))
        .await;
    if let Err(e) = result {
        error!("appending row failed for message {message_id} after extended retry: {e:#}");
    }
}

async fn build_entry(gcs: &GcsClient, job: &Job) -> Result<ResultsEntry> {
    let mut entry = ResultsEntry::new(
        job.jump_url.clone(),
        job.poster.clone(),
        job.content.clone(),
    );
    entry.bracket = job.category.clone();

    parse_message_content(&mut entry, &job.content);

    if let Some(id) = entry.player1_id {
        entry.player1_name = Some(fetch_display_name(&job.http, UserId::new(id)).await);
    }
    if let Some(id) = entry.player2_id {
        entry.player2_name = Some(fetch_display_name(&job.http, UserId::new(id)).await);
    }

    let mut download_links = Vec::with_capacity(job.attachments.len());
    for (idx, attachment) in job.attachments.iter().enumerate() {
        let object_name = format!(
            "{}{}_{}",
            job.gcs_prefix, attachment.id, attachment.filename
        );
        // Discord does not allow adding or changing attachments on an
        // existing message, so on an edit the files were already uploaded
        // by the original `message` event. Re-uploading would overwrite the
        // existing object, which GCS treats as a delete+create and rejects
        // for a create-only service account. Reuse the deterministic name
        // so the row still carries a complete replays_link.
        if job.event == MessageEvent::Updated {
            debug!(
                "Skipping upload of attachment {} on message edit; reusing {}",
                idx + 1,
                object_name
            );
        } else {
            // Retried as a single unit up to the extended job horizon: since
            // nothing persists until `gcs.upload` inside it succeeds,
            // re-entering this closure from scratch is always safe, and it's
            // never re-entered again once it has succeeded — see module docs
            // on why this must stay scoped to one attachment at a time.
            (|| async {
                let bytes = attachment.download().await.with_context(|| {
                    format!(
                        "downloading attachment {} ({})",
                        attachment.id, attachment.filename
                    )
                })?;
                info!(
                    "Uploading attachment {} as {} with {} bytes of data",
                    idx + 1,
                    object_name,
                    bytes.len()
                );
                gcs.upload(&object_name, bytes).await
            })
            .retry(retry::job_backoff())
            .when(|_| true)
            .notify(retry::log_retry("worker.upload_attachment"))
            .await?;
        }
        download_links.push(format!("gcs://{}/{}", gcs.bucket(), object_name));
    }

    if !download_links.is_empty() {
        entry.replays_link = Some(download_links.join("\n"));
    } else {
        entry.replays_link = Some(String::new());
    }
    Ok(entry)
}

async fn fetch_display_name(http: &Arc<Http>, user_id: UserId) -> String {
    match user_id.to_user(http).await {
        Ok(user) => user.global_name.unwrap_or(user.name),
        Err(e) => {
            warn!("failed to fetch user {user_id}: {e}");
            user_id.to_string()
        }
    }
}
