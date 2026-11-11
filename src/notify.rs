use anyhow::Result;
use serde_json::json;

use crate::app::{App, BusEvent};
use crate::db::NewAlert;
use crate::model::{Alert, Asset};
use crate::util::now;

pub struct Draft {
    /// `scheduled` | `realtime` | `composite`
    pub tier: &'static str,
    /// `info` | `warn` | `strong`
    pub severity: &'static str,
    pub rule: &'static str,
    pub asset: Option<Asset>,
    pub title: String,
    pub body: String,
    /// Emitted at most once per key (e.g. one alert per data date).
    pub dedupe: Option<String>,
    /// Minimum seconds between alerts of the same rule and asset; 0 disables.
    pub cooldown: i64,
}

/// Persists the alert, pushes it to connected clients and to Telegram. Returns false when suppressed.
pub fn emit(app: &App, d: Draft) -> Result<bool> {
    let ts = now();
    let asset = d.asset.map(Asset::key);
    if d.cooldown > 0
        && let Some(last) = app.store.last_alert_ts(d.rule, asset)?
        && ts - last < d.cooldown
    {
        return Ok(false);
    }
    let Some(alert) = app.store.insert_alert(&NewAlert {
        ts,
        tier: d.tier,
        severity: d.severity,
        rule: d.rule,
        asset,
        title: &d.title,
        body: &d.body,
        dedupe: d.dedupe.as_deref(),
    })?
    else {
        return Ok(false);
    };
    tracing::info!(rule = d.rule, severity = d.severity, "alert: {}", d.title);
    let _ = app.bus.send(BusEvent::Alert(alert.clone()));
    if app.cfg.telegram.is_some() {
        tokio::spawn(telegram(app.clone(), alert));
    }
    Ok(true)
}

async fn telegram(app: App, alert: Alert) {
    let Some(tg) = &app.cfg.telegram else { return };
    // Plain text: the severity is already in the wording of the title.
    let res = app
        .http
        .client()
        .post(format!("https://api.telegram.org/bot{}/sendMessage", tg.token))
        .json(&json!({ "chat_id": tg.chat_id, "text": format!("{}\n{}", alert.title, alert.body) }))
        .send()
        .await
        .and_then(|r| r.error_for_status());
    match res {
        Ok(_) => app.status.ok("telegram"),
        Err(e) => {
            // The bot token is part of the URL; never log it.
            let msg = e.without_url().to_string();
            tracing::warn!("telegram: {msg}");
            app.status.err("telegram", msg);
        }
    }
}
