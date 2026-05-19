//! TEMPORARY diagnostic instrumentation.
//!
//! This module exists only to collect enough detail to file an upstream
//! serenity bug report about [`HttpError::UnsuccessfulRequest`] discarding the
//! HTTP status / method / URL when Discord returns a non-JSON error body
//! (observed as the opaque `[Serenity] Could not decode json when receiving
//! error response from discord:, expected value at line 1 column 1`).
//!
//! Once the report is filed (and ideally fixed/worked-around upstream), delete
//! this module and inline plain `error!("...: {e:#}")` calls again at the call
//! sites in [`crate::handler`].

use serenity::{http::HttpError, Error as SerenityError};
use tracing::error;

/// Pull the HTTP status, method and URL out of a serenity error chain.
///
/// serenity 0.12's [`HttpError::UnsuccessfulRequest`] `Display` only renders
/// `ErrorResponse.error.message` — the *body* Discord sent. When that body
/// isn't valid JSON (an empty 5xx, a Cloudflare HTML interstitial, a bare 429),
/// serenity can't decode it and substitutes an opaque placeholder, discarding
/// the status code, method and URL even though `ErrorResponse` still holds
/// them. Recover those fields by downcasting so a log line / bug report shows
/// *which* request failed and *how* (e.g. `HTTP 503 Service Unavailable`), not
/// just that JSON decoding failed.
fn discord_http_detail(err: &anyhow::Error) -> Option<String> {
    err.chain().find_map(|cause| {
        let SerenityError::Http(HttpError::UnsuccessfulRequest(resp)) =
            cause.downcast_ref::<SerenityError>()?
        else {
            return None;
        };
        Some(format!(
            "HTTP {} on {} {} (discord error code {}, body: {:?})",
            resp.status_code, resp.method, resp.url, resp.error.code, resp.error.message
        ))
    })
}

/// Render an anyhow chain like `{err:#}`, but collapse a frame whose `Display`
/// equals the previous frame's. `serenity::Error::Http`'s `Display` delegates
/// to its inner `HttpError` *and* its `source()` returns that same
/// `HttpError`, so the default `{:#}` walk prints the serenity message twice in
/// a row — the doubled text in the original report is this artifact, not two
/// distinct failures.
fn render_chain(err: &anyhow::Error) -> String {
    let mut out = String::new();
    let mut prev: Option<String> = None;
    for cause in err.chain() {
        let frame = cause.to_string();
        if prev.as_deref() == Some(frame.as_str()) {
            continue;
        }
        if !out.is_empty() {
            out.push_str(": ");
        }
        out.push_str(&frame);
        prev = Some(frame);
    }
    out
}

/// Log an `anyhow` failure at `error!`, enriched with recovered Discord HTTP
/// detail when the underlying cause is a serenity HTTP error (see
/// [`discord_http_detail`]).
pub fn log_processing_failure(context: &str, err: &anyhow::Error) {
    let chain = render_chain(err);
    match discord_http_detail(err) {
        Some(detail) => error!(discord_http = %detail, "{context}: {chain}"),
        None => error!("{context}: {chain}"),
    }
}

/// As [`log_processing_failure`], for a bare `serenity::Error` (not wrapped in
/// `anyhow`) — used where we call serenity directly without `with_context`.
pub fn log_serenity_failure(context: &str, err: &SerenityError) {
    if let SerenityError::Http(HttpError::UnsuccessfulRequest(resp)) = err {
        error!(
            discord_http = %format!(
                "HTTP {} on {} {} (discord error code {}, body: {:?})",
                resp.status_code, resp.method, resp.url, resp.error.code, resp.error.message
            ),
            "{context}: {err}"
        );
    } else {
        error!("{context}: {err}");
    }
}
