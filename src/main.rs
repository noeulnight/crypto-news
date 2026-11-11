mod api;
mod app;
mod brief;
mod claude;
mod collect;
mod config;
mod db;
mod http;
mod judge;
mod model;
mod notify;
mod signal;
mod util;

use std::sync::Arc;

use anyhow::Result;

use crate::app::App;
use crate::config::Config;

#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt().with_target(false).init();
    // reqwest and tokio-tungstenite both use rustls; pin one crypto provider for the process.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let cfg = Arc::new(Config::from_env());
    let app = App::new(cfg.clone())?;
    tracing::info!(
        claude = app.claude.is_some(),
        telegram = cfg.telegram.is_some(),
        whale_alert = app.whale_enabled(),
        "listening on http://127.0.0.1:{}",
        cfg.port
    );

    let jobs = app.clone();
    tokio::spawn(async move {
        collect::spawn_all(&jobs);
        // Rules and judgments wait for history, otherwise early z-scores would be noise.
        collect::backfill(&jobs).await;
        signal::rules::spawn(&jobs);
        judge::spawn(&jobs);
        brief::spawn(&jobs);
    });

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", cfg.port)).await?;
    axum::serve(listener, api::router(app)).await?;
    Ok(())
}
