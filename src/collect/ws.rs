//! Shared WebSocket plumbing for the streaming collectors.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use futures_util::{SinkExt, StreamExt};
use tokio::time::{interval, sleep};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use crate::app::App;

/// Connects, optionally subscribes, and feeds every text frame to `on_text` until the stream fails.
/// `ping` is an application-level keepalive: (period in seconds, payload).
pub async fn run_ws(
    app: &App,
    name: &'static str,
    url: &str,
    subscribe: Option<String>,
    ping: Option<(u64, &'static str)>,
    mut on_text: impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    let (ws, _) = connect_async(url)
        .await
        .with_context(|| format!("{name}: connect"))?;
    let (mut tx, mut rx) = ws.split();
    if let Some(msg) = subscribe {
        tx.send(Message::text(msg)).await?;
    }
    app.status.ok(name);

    let mut ping_tick = interval(Duration::from_secs(ping.map_or(3600, |p| p.0)));
    ping_tick.tick().await;
    loop {
        tokio::select! {
            frame = rx.next() => match frame.context("stream ended")?? {
                Message::Text(text) => {
                    on_text(text.as_str())?;
                    app.status.ok(name);
                }
                Message::Close(_) => bail!("closed by server"),
                _ => {}
            },
            _ = ping_tick.tick() => {
                if let Some((_, payload)) = ping {
                    tx.send(Message::text(payload)).await?;
                }
            }
        }
    }
}

/// Keeps a streaming collector alive: reconnects 5s after any failure.
pub fn supervise<F, Fut>(app: &App, name: &'static str, f: F)
where
    F: Fn(App) -> Fut + Send + 'static,
    Fut: Future<Output = Result<()>> + Send + 'static,
{
    let app = app.clone();
    tokio::spawn(async move {
        loop {
            let msg = match f(app.clone()).await {
                Ok(()) => "stream ended".to_owned(),
                Err(e) => format!("{e:#}"),
            };
            tracing::warn!(collector = name, "{msg}");
            app.status.err(name, msg);
            sleep(Duration::from_secs(5)).await;
        }
    });
}
