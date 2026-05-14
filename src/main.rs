use std::{path::PathBuf, sync::Arc};

use anyhow::{anyhow, Context, Result};
use serenity::{all::GatewayIntents, Client};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

mod config;
mod entry;
mod gcs;
mod handler;
mod parse;
mod sheets;
mod tournament;

use crate::{config::Config, gcs::GcsClient, handler::Handler, sheets::SheetsClient};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config_path = std::env::var_os("CONFIG_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("config.toml"));
    info!("Loading config from {}", config_path.display());
    let config = Arc::new(Config::load(&config_path)?);

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
    let existing_tabs = sheets
        .list_tabs()
        .await
        .context("listing spreadsheet tabs")?;
    let missing: Vec<&str> = configured_tabs
        .iter()
        .copied()
        .filter(|t| !existing_tabs.iter().any(|e| e == *t))
        .collect();
    if !missing.is_empty() {
        return Err(anyhow!(
            "tournaments reference sheet tabs that do not exist: {:?} (have: {:?})",
            missing,
            existing_tabs
        ));
    }
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
