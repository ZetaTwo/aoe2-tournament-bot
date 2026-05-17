use std::{path::PathBuf, sync::Arc};

use anyhow::{Context, Result};
use serenity::{all::GatewayIntents, http::Http, Client};
use tracing::{info, warn, Level};
use tracing_subscriber::{fmt, prelude::*, registry, reload, EnvFilter};

mod config;
mod entry;
mod gcs;
mod handler;
mod notify;
mod parse;
mod retry;
mod sheets;
mod tournament;

use crate::{
    config::Config, gcs::GcsClient, handler::Handler, notify::DiscordErrorLayer,
    sheets::SheetsClient,
};

#[tokio::main]
async fn main() -> Result<()> {
    // rustls 0.23 has both `ring` and `aws-lc-rs` compiled in (pulled by
    // google-sheets4 and reqwest respectively), so it can't auto-select a
    // process-level CryptoProvider. Pin it before any TLS handshake.
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("install rustls ring CryptoProvider");

    // Basic tracing first; the Discord error layer starts inert (`None`) and is
    // swapped in via the reload handle once config (and the token) is loaded.
    let (discord_layer, discord_handle) = reload::Layer::new(None::<DiscordErrorLayer>);
    registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(fmt::layer())
        .with(discord_layer)
        .init();

    let config_path = std::env::var_os("CONFIG_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("config.toml"));
    let tournaments_path = std::env::var_os("TOURNAMENTS_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("tournaments.toml"));
    info!(
        "Loading config from {} and tournaments from {}",
        config_path.display(),
        tournaments_path.display()
    );
    let config = Arc::new(Config::load(&config_path, &tournaments_path)?);

    // Now that the token is known, activate Discord forwarding of ERROR logs.
    let error_http = Arc::new(Http::new(&config.bot.discord_token));
    discord_handle
        .reload(Some(DiscordErrorLayer::new(
            error_http,
            config.bot.admin_user_ids.clone(),
            Level::ERROR,
        )))
        .expect("install Discord error log layer");

    info!("Replays will be saved to bucket \"{}\"", config.gcp.bucket);
    info!(
        "Errors will be sent to user IDs {:?}",
        config.bot.admin_user_ids
    );

    let sheets = Arc::new(
        SheetsClient::new(config.gcp.sheet_id.clone())
            .await
            .context("constructing Sheets client")?,
    );
    let configured_tabs: Vec<&str> = config
        .tournaments
        .iter()
        .map(|t| t.sheet_tab.as_str())
        .collect();
    sheets
        .ensure_tabs(&configured_tabs)
        .await
        .context("ensuring tournament tabs exist")?;
    info!(
        "Results sheet set up; {} tournament tab(s) verified",
        configured_tabs.len()
    );

    let gcs = Arc::new(
        GcsClient::new(config.gcp.bucket.clone())
            .await
            .context("constructing GCS client")?,
    );

    let token = config.bot.discord_token.clone();
    let intents = GatewayIntents::GUILD_MESSAGES | GatewayIntents::MESSAGE_CONTENT;
    let handler = Handler {
        config: config.clone(),
        sheets,
        gcs,
    };
    let mut client = Client::builder(token, intents)
        .event_handler(handler)
        .await
        .context("building Discord client")?;

    info!("Connecting to Discord...");
    if let Err(e) = client.start().await {
        warn!("Discord client exited with error: {e:#}");
        return Err(e.into());
    }
    info!("Shutting down...");
    Ok(())
}
