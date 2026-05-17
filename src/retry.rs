//! Shared retry policy for the Sheets and GCS network clients.
//!
//! Transient failures (notably HTTP 503 from the Sheets API) should not lose a
//! tournament result. Every network call is wrapped with [`backon`] using the
//! single [`backoff`] policy below, the matching `*_retryable` predicate so we
//! only retry transient errors, and [`log_retry`] so each attempt is visible.
//!
//! `append_row` (Sheets values.append) is deliberately retried even though it
//! is not idempotent: a 503 returned to the client may still have committed
//! the row, so a retry can rarely create a duplicate. A duplicate row is
//! visible and hand-deletable; silently losing a result is worse.

use std::time::Duration;

use backon::ExponentialBuilder;
use tracing::warn;

/// ~5 total attempts (1 + 4 retries) with a slight exponential backoff plus
/// jitter. Base delays before retries 2..5 are 200ms / 400ms / 800ms / 1.6s;
/// worst-case ≈3–6 s of added latency before a permanent error surfaces.
pub fn backoff() -> ExponentialBuilder {
    ExponentialBuilder::default()
        .with_min_delay(Duration::from_millis(200))
        .with_factor(2.0)
        .with_max_delay(Duration::from_secs(5))
        .with_max_times(4)
        .with_jitter()
}

/// Retry predicate for the GCS client.
pub fn gcs_retryable(e: &gcloud_storage::http::Error) -> bool {
    use gcloud_storage::http::Error;
    match e {
        // `is_retriable()` is `matches!(code, 408 | 429 | 500..=599)` — covers 503.
        Error::Response(r) => r.is_retriable(),
        Error::HttpClient(e) | Error::RawResponse(e, _) => e.is_timeout() || e.is_connect(),
        _ => false,
    }
}

/// Retry predicate for the Sheets client.
pub fn sheets_retryable(e: &google_sheets4::Error) -> bool {
    use google_sheets4::hyper::StatusCode;
    use google_sheets4::Error;
    match e {
        // hyper transport-level failure (connection reset / timeout).
        Error::HttpError(_) => true,
        Error::Failure(resp) => {
            let s = resp.status();
            s == StatusCode::REQUEST_TIMEOUT
                || s == StatusCode::TOO_MANY_REQUESTS
                || s.is_server_error()
        }
        _ => false,
    }
}

/// `.notify(...)` callback: log every retry at WARN with a short op label.
pub fn log_retry<E: std::fmt::Display>(op: &'static str) -> impl FnMut(&E, Duration) {
    move |err, delay| {
        warn!("{op} failed ({err}); retrying in {delay:?}");
    }
}
