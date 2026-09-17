use std::{path::PathBuf, sync::Arc};

use anyhow::{Context, Result};
use serenity::{all::GatewayIntents, Client};
use tracing::{info, warn};
use tracing_subscriber::{fmt, prelude::*, registry, EnvFilter};

mod config;
mod diag;
mod entry;
mod gcs;
mod handler;
mod parse;
mod retry;
mod sheets;
mod tournament;
mod worker;

use crate::{config::Config, gcs::GcsClient, handler::Handler, sheets::SheetsClient};

#[tokio::main]
async fn main() -> Result<()> {
    // rustls 0.23 has both `ring` and `aws-lc-rs` compiled in (pulled by
    // google-sheets4 and reqwest respectively), so it can't auto-select a
    // process-level CryptoProvider. Pin it before any TLS handshake.
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("install rustls ring CryptoProvider");

    // Structured JSON logs to stdout; infra tails and alerts on these.
    registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(fmt::layer().json().flatten_event(true))
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

    info!("Replays will be saved to bucket \"{}\"", config.gcp.bucket);

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

    let (job_tx, job_rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(worker::run(job_rx, sheets, gcs));

    let token = config.bot.discord_token.clone();
    let intents = GatewayIntents::GUILD_MESSAGES | GatewayIntents::MESSAGE_CONTENT;
    let handler = Handler {
        config: config.clone(),
        job_tx,
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
