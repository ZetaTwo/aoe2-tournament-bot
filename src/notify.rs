//! A `tracing-subscriber` layer that forwards log events at/above a configured
//! severity to the bot's admin users as Discord DMs.
//!
//! `Layer::on_event` is synchronous but a Discord send is async, so the send is
//! `tokio::spawn`ed (we are always inside the `#[tokio::main]` runtime). The
//! layer is installed early via `tracing_subscriber::reload` as an inert `None`
//! and the real layer is swapped in once the config (and thus the bot token) is
//! available — see `main.rs`.
//!
//! Feedback-loop safety: a forwarded send runs through serenity's REST client,
//! which logs under the `serenity*` target, and this module's own failures log
//! under this module's target. Both targets are excluded in `on_event`, so a
//! send failure can never produce another forwarded event — independent of the
//! configured level.

use std::fmt::Debug;
use std::sync::Arc;

use chrono::{SecondsFormat, Utc};
use serenity::{all::UserId, http::Http};
use tracing::{
    field::{Field, Visit},
    warn, Event, Level, Subscriber,
};
use tracing_subscriber::layer::{Context, Layer};

/// Discord message content hard limit (characters; bytes is a safe upper bound).
const DISCORD_LIMIT: usize = 2000;

pub struct DiscordErrorLayer {
    http: Arc<Http>,
    admins: Arc<[u64]>,
    /// Minimum severity to forward. `tracing::Level` orders `ERROR` lowest, so
    /// "at least as severe as `level`" is `event_level <= level`.
    level: Level,
}

impl DiscordErrorLayer {
    pub fn new(http: Arc<Http>, admins: Vec<u64>, level: Level) -> Self {
        Self {
            http,
            admins: admins.into(),
            level,
        }
    }
}

/// `true` if an event at `event` severity should be forwarded given `threshold`.
/// `tracing::Level`'s `Ord` is inverted (ERROR is the lowest), so a more-or-
/// equally-severe event compares `<=` the threshold.
fn level_enabled(event: &Level, threshold: &Level) -> bool {
    event <= threshold
}

#[derive(Default)]
struct MessageVisitor {
    message: Option<String>,
    fields: Vec<(String, String)>,
}

impl MessageVisitor {
    fn put(&mut self, field: &Field, value: String) {
        if field.name() == "message" {
            self.message = Some(value);
        } else {
            self.fields.push((field.name().to_string(), value));
        }
    }
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        self.put(field, format!("{value:?}"));
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        self.put(field, value.to_string());
    }
    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        self.put(field, value.to_string());
    }
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.put(field, value.to_string());
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.put(field, value.to_string());
    }
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.put(field, value.to_string());
    }
}

/// Build the Discord message: a metadata header line, then the message and any
/// structured fields in a fenced code block, truncated to fit `DISCORD_LIMIT`.
pub(crate) fn format_body(
    level: &Level,
    target: &str,
    message: &str,
    fields: &[(String, String)],
) -> String {
    let ts = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
    let header = format!("🔴 {level} · {target} · {ts}");

    let mut core = String::from(message);
    for (k, v) in fields {
        if !core.is_empty() {
            core.push('\n');
        }
        core.push_str(&format!("{k}={v}"));
    }

    // Final layout: `{header}\n```\n{core}\n````. Reserve everything but core.
    let marker = "\n… (truncated)";
    let overhead = header.len() + "\n```\n".len() + "\n```".len();
    let avail = DISCORD_LIMIT.saturating_sub(overhead);
    if core.len() > avail {
        let mut end = avail.saturating_sub(marker.len()).min(core.len());
        while end > 0 && !core.is_char_boundary(end) {
            end -= 1;
        }
        core.truncate(end);
        core.push_str(marker);
    }

    format!("{header}\n```\n{core}\n```")
}

impl<S: Subscriber> Layer<S> for DiscordErrorLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let meta = event.metadata();
        if !level_enabled(meta.level(), &self.level) {
            return;
        }
        let target = meta.target();
        // Break the feedback loop: serenity's REST client and this module's own
        // failures must never be re-forwarded.
        if target.starts_with("serenity") || target == module_path!() {
            return;
        }

        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);
        let body = format_body(
            meta.level(),
            target,
            visitor.message.as_deref().unwrap_or_default(),
            &visitor.fields,
        );

        // on_event is sync; the send is async. Never block here.
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let http = Arc::clone(&self.http);
        let admins = Arc::clone(&self.admins);
        handle.spawn(async move {
            for &id in admins.iter() {
                let uid = UserId::new(id);
                match uid.create_dm_channel(&http).await {
                    Ok(dm) => {
                        if let Err(e) = dm.id.say(&http, body.as_str()).await {
                            warn!("failed to DM admin {id}: {e}");
                        }
                    }
                    Err(e) => warn!("failed to open DM channel for admin {id}: {e}"),
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_threshold_error_forwards_only_error() {
        assert!(level_enabled(&Level::ERROR, &Level::ERROR));
        assert!(!level_enabled(&Level::WARN, &Level::ERROR));
        assert!(!level_enabled(&Level::INFO, &Level::ERROR));
    }

    #[test]
    fn level_threshold_warn_forwards_error_and_warn() {
        assert!(level_enabled(&Level::ERROR, &Level::WARN));
        assert!(level_enabled(&Level::WARN, &Level::WARN));
        assert!(!level_enabled(&Level::INFO, &Level::WARN));
    }

    #[test]
    fn format_body_has_header_and_fenced_block() {
        let body = format_body(&Level::ERROR, "aoe2_tournament_bot::handler", "boom", &[]);
        assert!(body.starts_with("🔴 ERROR · aoe2_tournament_bot::handler · "));
        assert!(body.contains("\n```\nboom\n```"));
    }

    #[test]
    fn format_body_appends_fields() {
        let fields = vec![("id".to_string(), "42".to_string())];
        let body = format_body(&Level::ERROR, "t", "msg", &fields);
        assert!(body.contains("msg\nid=42"));
    }

    #[test]
    fn format_body_truncates_to_discord_limit_on_char_boundary() {
        // Multi-byte payload larger than the limit must not panic and must fit.
        let huge = "é".repeat(4000);
        let body = format_body(&Level::ERROR, "t", &huge, &[]);
        assert!(body.len() <= DISCORD_LIMIT);
        assert!(body.contains("… (truncated)"));
        // Still valid UTF-8 / no broken char (String guarantees this; explicit
        // round-trip guards against a bad byte-boundary slice regressing).
        assert_eq!(body, String::from_utf8(body.clone().into_bytes()).unwrap());
    }

    #[test]
    fn format_body_short_message_not_truncated() {
        let body = format_body(&Level::ERROR, "t", "short", &[]);
        assert!(!body.contains("truncated"));
    }
}
