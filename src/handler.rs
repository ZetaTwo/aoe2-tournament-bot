use std::sync::Arc;

use anyhow::{Context as _, Result};
use chrono::{SecondsFormat, Utc};
use serenity::{
    all::{
        Channel, ChannelType, Context, EventHandler, GuildChannel, Message, MessageUpdateEvent,
        Ready, UserId,
    },
    async_trait,
};
use tracing::{debug, error, info, warn};

use crate::{
    config::{Config, Tournament},
    entry::ResultsEntry,
    gcs::GcsClient,
    parse::parse_message_content,
    sheets::SheetsClient,
    tournament::{match_tournament, MatchInput},
};

pub struct Handler {
    pub config: Arc<Config>,
    pub sheets: Arc<SheetsClient>,
    pub gcs: Arc<GcsClient>,
}

/// Which gateway event delivered the message. On [`MessageEvent::Updated`],
/// Discord guarantees the attachments are unchanged from the original post,
/// so the files are already in GCS and must not be re-uploaded — an overwrite
/// needs `storage.objects.delete`, which the runtime service account lacks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MessageEvent {
    Created,
    Updated,
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
            error!("processing message failed: {e:#}");
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
        let message = match new {
            Some(m) => m,
            None => match ctx.http.get_message(event.channel_id, event.id).await {
                Ok(m) => m,
                Err(e) => {
                    error!("fetching updated message {}: {e}", event.id);
                    return;
                }
            },
        };
        if let Err(e) = self
            .process_message(&ctx, message, MessageEvent::Updated)
            .await
        {
            error!("processing updated message failed: {e:#}");
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
            guild_id: message.guild_id.map(|g| g.get()),
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
            "processing as a results message",
        );

        let entry = self
            .construct_results_entry(
                ctx,
                &message,
                &channel,
                category.as_deref(),
                tournament,
                event,
            )
            .await?;

        let now = Utc::now();
        let mut row = Vec::with_capacity(crate::entry::SHEET_COLUMN_COUNT);
        row.push(now.to_rfc3339_opts(SecondsFormat::Secs, false));
        row.extend(entry.get_row());

        if let Err(e) = self.sheets.append_row(&tournament.sheet_tab, row).await {
            error!("appending row failed for message {}: {e:#}", message.id);
            let admin_msg = format!(
                "AoE2 Tournament Bot error: failed to append results row for message {}. Please check logs",
                message.id
            );
            self.notify_admins(ctx, &admin_msg).await;
        }

        Ok(())
    }

    async fn construct_results_entry(
        &self,
        ctx: &Context,
        message: &Message,
        channel: &GuildChannel,
        category: Option<&str>,
        tournament: &Tournament,
        event: MessageEvent,
    ) -> Result<ResultsEntry> {
        let jump_url = message.link();
        let poster = message
            .author
            .global_name
            .clone()
            .unwrap_or_else(|| message.author.name.clone());

        let mut entry = ResultsEntry::new(jump_url, poster, message.content.clone());
        entry.bracket = category.map(|s| s.to_string());

        parse_message_content(&mut entry, &message.content);

        if let Some(id) = entry.player1_id {
            entry.player1_name = Some(fetch_display_name(ctx, UserId::new(id)).await);
        }
        if let Some(id) = entry.player2_id {
            entry.player2_name = Some(fetch_display_name(ctx, UserId::new(id)).await);
        }

        let mut download_links = Vec::with_capacity(message.attachments.len());
        for (idx, attachment) in message.attachments.iter().enumerate() {
            let object_name = format!(
                "{}{}_{}",
                tournament.gcs_prefix, attachment.id, attachment.filename
            );
            // Discord does not allow adding or changing attachments on an
            // existing message, so on an edit the files were already uploaded
            // by the original `message` event. Re-uploading would overwrite the
            // existing object, which GCS treats as a delete+create and rejects
            // for a create-only service account. Reuse the deterministic name
            // so the row still carries a complete replays_link.
            if event == MessageEvent::Updated {
                debug!(
                    "Skipping upload of attachment {} on message edit; reusing {}",
                    idx + 1,
                    object_name
                );
            } else {
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
                self.gcs.upload(&object_name, bytes).await?;
            }
            download_links.push(format!("gcs://{}/{}", self.gcs.bucket(), object_name));
        }

        if !download_links.is_empty() {
            entry.replays_link = Some(download_links.join("\n"));
        } else {
            entry.replays_link = Some(String::new());
        }
        let _ = channel;
        Ok(entry)
    }

    async fn notify_admins(&self, ctx: &Context, body: &str) {
        for &admin_id in &self.config.bot.admin_user_ids {
            let user_id = UserId::new(admin_id);
            match user_id.create_dm_channel(&ctx.http).await {
                Ok(dm) => {
                    if let Err(e) = dm.id.say(&ctx.http, body).await {
                        warn!("failed to DM admin {admin_id}: {e}");
                    }
                }
                Err(e) => warn!("failed to open DM channel for admin {admin_id}: {e}"),
            }
        }
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

async fn fetch_display_name(ctx: &Context, user_id: UserId) -> String {
    match user_id.to_user(&ctx.http).await {
        Ok(user) => user.global_name.unwrap_or(user.name),
        Err(e) => {
            warn!("failed to fetch user {user_id}: {e}");
            user_id.to_string()
        }
    }
}
