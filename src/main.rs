mod api;
mod config;
mod db;
mod models;
mod probe;
mod scheduler;
mod status;
mod version;

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{RwLock, broadcast, watch};
use tracing_subscriber::EnvFilter;

use crate::api::AppState;
use crate::config::{AppConfig, CliOptions};
use crate::scheduler::{Scheduler, cleanup_loop};
use crate::version::{APP_NAME, VERSION};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "pinginfo=info,tower_http=info".into()),
        )
        .init();

    let cli = CliOptions::parse()?;
    if cli.show_help {
        CliOptions::print_usage();
        return Ok(());
    }
    if cli.show_version {
        println!("{APP_NAME} {VERSION}");
        return Ok(());
    }
    let config = AppConfig::from_env(&cli)?;
    let pool = db::connect(&config.database_path).await?;
    let statuses = Arc::new(RwLock::new(HashMap::new()));
    let (events, _) = broadcast::channel(512);
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let scheduler = Scheduler::new(pool.clone(), statuses.clone(), events.clone());
    scheduler.start_existing_targets().await?;

    tokio::spawn(cleanup_loop(pool.clone(), config.retention_days));

    let state = AppState::with_shutdown(pool, statuses, scheduler.clone(), events, shutdown_rx);
    let app = api::router(state);
    let listener = tokio::net::TcpListener::bind(config.bind).await?;
    tracing::info!(version = VERSION, addr = %config.bind, "{APP_NAME} listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(scheduler, shutdown_tx))
        .await?;
    Ok(())
}

async fn shutdown_signal(scheduler: Scheduler, shutdown_tx: watch::Sender<bool>) {
    let _ = tokio::signal::ctrl_c().await;
    let _ = shutdown_tx.send(true);
    scheduler.stop_all().await;
}
