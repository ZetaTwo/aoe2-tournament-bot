use std::sync::Arc;

use anyhow::{anyhow, Context as _, Result};
use serenity::{
    all::{
        Channel, ChannelType, Context, EventHandler, GuildChannel, Message, MessageUpdateEvent,
        Ready,
    },
    async_trait,
};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::{
    config::Config,
    diag::{log_processing_failure, log_serenity_failure},
    tournament::{match_tournament, MatchInput},
    worker::{Job, MessageEvent},
};

pub struct Handler {
    pub config: Arc<Config>,
    pub job_tx: mpsc::UnboundedSender<Job>,
}

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, _ctx: Context, ready: Ready) {
        info!("Logged in as {}", ready.user.name);
    }

    async fn message(&self, ctx: Context, message: Message) {
        debug!(id = %message.id, "processing new message");
        if let Err(e) = self
            .process_message(&ctx, message, MessageEvent::Created)
            .await
        {
            log_processing_failure("processing message failed", &e);
        }
    }

    async fn message_update(
        &self,
        ctx: Context,
        _old: Option<Message>,
        new: Option<Message>,
        event: MessageUpdateEvent,
    ) {
        debug!(id = %event.id, "processing updated message");
        // Discord fires message_update for its own link/embed-preview unfurls,
        // not just human edits. Only a real content edit sets edited_timestamp
        // (an unfurl leaves it null), so skip everything else — otherwise the
        // unfurl that follows almost every results post would append a
        // duplicate row.
        if event.edited_timestamp.is_none() {
            debug!(id = %event.id, "skipping non-edit update (no edited_timestamp)");
            return;
        }
        let message = match new {
            Some(m) => m,
            None => match ctx.http.get_message(event.channel_id, event.id).await {
                Ok(m) => m,
                Err(e) => {
                    log_serenity_failure(&format!("fetching updated message {}", event.id), &e);
                    return;
                }
            },
        };
        if let Err(e) = self
            .process_message(&ctx, message, MessageEvent::Updated)
            .await
        {
            log_processing_failure("processing updated message failed", &e);
        }
    }
}

impl Handler {
    async fn process_message(
        &self,
        ctx: &Context,
        message: Message,
        event: MessageEvent,
    ) -> Result<()> {
        if message.author.id == ctx.cache.current_user().id {
            return Ok(());
        }

        let (channel, category) = match resolve_channel(ctx, &message).await? {
            Some(c) => c,
            None => return Ok(()),
        };

        let input = MatchInput {
            guild_id: channel.guild_id.get(),
            channel_name: channel.name.as_str(),
            category: category.as_deref(),
        };
        let tournament = match match_tournament(&self.config.tournaments, input) {
            Some(t) => t,
            None => return Ok(()),
        };

        info!(
            id = %message.id,
            tournament = %tournament.name,
            guild = input.guild_id,
            category = ?input.category,
            channel = input.channel_name,
            "matched results message to tournament",
        );

        let poster = message
            .author
            .global_name
            .clone()
            .unwrap_or_else(|| message.author.name.clone());
        let job = Job {
            http: ctx.http.clone(),
            message_id: message.id,
            jump_url: message.link(),
            poster,
            content: message.content.clone(),
            category,
            gcs_prefix: tournament.gcs_prefix.clone(),
            sheet_tab: tournament.sheet_tab.clone(),
            attachments: message.attachments.clone(),
            event,
        };
        // Unbounded, so this never blocks the gateway task; the send only
        // fails if the worker task itself has exited (e.g. panicked), which
        // is a process-level problem this error surfaces via the normal
        // log_processing_failure path.
        self.job_tx
            .send(job)
            .map_err(|_| anyhow!("background worker queue is closed"))?;

        Ok(())
    }
}

async fn resolve_channel(
    ctx: &Context,
    message: &Message,
) -> Result<Option<(GuildChannel, Option<String>)>> {
    let channel = message
        .channel_id
        .to_channel(&ctx.http)
        .await
        .with_context(|| format!("fetching channel {}", message.channel_id))?;
    let guild_channel = match channel {
        Channel::Guild(g) if g.kind == ChannelType::Text => g,
        _ => return Ok(None),
    };

    let category = match guild_channel.parent_id {
        Some(parent_id) => match parent_id.to_channel(&ctx.http).await {
            Ok(Channel::Guild(parent)) if parent.kind == ChannelType::Category => Some(parent.name),
            Ok(_) => None,
            Err(e) => {
                warn!("failed to fetch parent category {parent_id}: {e}");
                None
            }
        },
        None => None,
    };

    Ok(Some((guild_channel, category)))
}
