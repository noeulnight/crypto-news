//! JSON API and static frontend.

use std::convert::Infallible;
use std::path::Path;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::{Stream, StreamExt};
use tower_http::services::{ServeDir, ServeFile};

use crate::app::{App, BusEvent};
use crate::db::Store;
use crate::judge;
use crate::model::Asset;
use crate::signal::catalog::{self, Basis, Cadence, Category, Def, Kind};
use crate::signal::snapshot::{Z_WINDOW, hourly_sums};
use crate::signal::stats::{lagged_changes, rolling_z};
use crate::util::{DAY, HOUR, now, pct};

pub struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

impl<E: Into<anyhow::Error>> From<E> for ApiError {
    fn from(e: E) -> Self {
        let e = e.into();
        tracing::error!("api: {e:#}");
        Self(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}"))
    }
}

fn bad(msg: &str) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, msg.into())
}

type ApiResult = Result<Json<Value>, ApiError>;

pub fn router(app: App) -> Router {
    let api = Router::new()
        .route("/overview", get(overview))
        .route("/series", get(series))
        .route("/news", get(news))
        .route("/events", get(events))
        .route("/alerts", get(alerts))
        .route("/judgments", get(judgments))
        .route("/ledger", get(ledger))
        .route("/brief", get(brief))
        .route("/status", get(status))
        .route("/stream", get(stream))
        .fallback(|| async { ApiError(StatusCode::NOT_FOUND, "no such endpoint".into()) })
        .with_state(app.clone());
    let router = Router::new().nest("/api", api);

    // The built frontend is optional: in development Vite serves it and proxies /api here.
    let index = Path::new(&app.cfg.web_dir).join("index.html");
    if index.exists() {
        router.fallback_service(ServeDir::new(&app.cfg.web_dir).fallback(ServeFile::new(index)))
    } else {
        tracing::info!("{} not found; serving the API only", index.display());
        router
    }
}

#[derive(Deserialize)]
struct AssetQuery {
    asset: Option<String>,
}

fn asset_of(q: Option<&str>) -> Result<Asset, ApiError> {
    Asset::parse(q.unwrap_or("btc")).ok_or_else(|| bad("asset must be btc or eth"))
}

async fn overview(State(app): State<App>, Query(q): Query<AssetQuery>) -> ApiResult {
    let asset = asset_of(q.asset.as_deref())?;
    let snap = app.snapshot(asset)?;
    let latest = |source: &str| {
        app.store
            .judgments(asset.key(), Some(source), 1)
            .map(|mut j| j.pop())
    };
    Ok(Json(json!({
        "asset": asset.key(),
        "ts": snap.ts,
        "price": snap.price,
        "chg_1h_pct": snap.chg_1h_pct,
        "chg_24h_pct": snap.chg_24h_pct,
        "claude_enabled": app.claude.is_some(),
        "scores": snap.scores,
        "judgments": { "claude": latest("claude")?, "rules": latest("rules")? },
        "metrics": snap.metrics,
    })))
}

#[derive(Deserialize)]
struct SeriesQuery {
    metric: String,
    hours: Option<i64>,
    mode: Option<String>,
}

/// Chart series for a metric at a resolution suited to its cadence.
fn load(
    store: &Store,
    def: Option<&Def>,
    metric: &str,
    from: i64,
    to: i64,
    hourly: bool,
) -> anyhow::Result<Vec<(i64, f64)>> {
    match def {
        Some(d) if d.kind == Kind::Sum1h => hourly_sums(store, metric, from, to),
        Some(d) if d.cadence != Cadence::Intraday => store.range(metric, from, to),
        _ => {
            let span = to - from;
            // Keep responses near a thousand points.
            let step = if hourly || span > 48 * HOUR {
                HOUR
            } else {
                ((span / 1000 + 59) / 60).max(1) * 60
            };
            store.bucket_last(metric, from, to, step)
        }
    }
}

async fn series(State(app): State<App>, Query(q): Query<SeriesQuery>) -> ApiResult {
    let hours = q.hours.unwrap_or(168).clamp(1, 2160);
    let z_mode = match q.mode.as_deref() {
        None | Some("raw") => false,
        Some("z") => true,
        Some(_) => return Err(bad("mode must be raw or z")),
    };
    let now = now();
    let from = now - hours * HOUR;
    let def = catalog::find(&q.metric);

    let points = if z_mode {
        // Same basis as the snapshot z-score: levels, or day changes for trending series.
        let series = load(
            &app.store,
            def,
            &q.metric,
            from - Z_WINDOW - 2 * DAY,
            now,
            true,
        )?;
        let daily = def.is_some_and(|d| d.cadence != Cadence::Intraday);
        let base = match def.map_or(Basis::Level, |d| d.basis) {
            Basis::Level => series,
            basis => {
                let relative = matches!(basis, Basis::ChgPct(_));
                if daily {
                    series
                        .windows(2)
                        .filter_map(|w| {
                            Some((
                                w[1].0,
                                if relative {
                                    pct(w[1].1, w[0].1)?
                                } else {
                                    w[1].1 - w[0].1
                                },
                            ))
                        })
                        .collect()
                } else {
                    let tol = if def.is_some_and(|d| d.category == Category::Macro) {
                        4 * DAY
                    } else {
                        3 * HOUR
                    };
                    lagged_changes(&series, DAY, tol, relative)
                }
            }
        };
        rolling_z(&base, Z_WINDOW, if daily { 10 } else { 24 })
            .into_iter()
            .filter(|p| p.0 >= from)
            .collect()
    } else {
        load(&app.store, def, &q.metric, from, now, false)?
    };
    Ok(Json(
        json!({ "metric": q.metric, "mode": if z_mode { "z" } else { "raw" }, "points": points }),
    ))
}

#[derive(Deserialize)]
struct NewsQuery {
    limit: Option<i64>,
    before_id: Option<i64>,
}

async fn news(State(app): State<App>, Query(q): Query<NewsQuery>) -> ApiResult {
    let now = now();
    let mut items = app
        .store
        .news(q.limit.unwrap_or(50).clamp(1, 100), q.before_id)?;
    let price = Asset::Btc.m("price");
    for n in &mut items {
        let Some((_, base)) = app.store.at_or_before(&price, n.ts, 5 * 60)? else {
            continue;
        };
        let react = |after: i64, tol: i64| -> anyhow::Result<Option<f64>> {
            // Not measurable until the window has fully elapsed.
            if now < n.ts + after {
                return Ok(None);
            }
            Ok(app
                .store
                .at_or_before(&price, n.ts + after, tol)?
                .and_then(|(_, p)| pct(p, base)))
        };
        n.react_5m_pct = react(5 * 60, 2 * 60)?;
        n.react_1h_pct = react(HOUR, 10 * 60)?;
    }
    Ok(Json(json!(items)))
}

#[derive(Deserialize)]
struct EventsQuery {
    days: Option<i64>,
}

async fn events(State(app): State<App>, Query(q): Query<EventsQuery>) -> ApiResult {
    let now = now();
    let days = q.days.unwrap_or(14).clamp(1, 90);
    Ok(Json(json!(app.store.events(now - DAY, now + days * DAY)?)))
}

#[derive(Deserialize)]
struct LimitQuery {
    limit: Option<i64>,
}

async fn alerts(State(app): State<App>, Query(q): Query<LimitQuery>) -> ApiResult {
    Ok(Json(json!(
        app.store.alerts(q.limit.unwrap_or(50).clamp(1, 200))?
    )))
}

#[derive(Deserialize)]
struct JudgmentsQuery {
    asset: Option<String>,
    source: Option<String>,
    limit: Option<i64>,
}

async fn judgments(State(app): State<App>, Query(q): Query<JudgmentsQuery>) -> ApiResult {
    let asset = asset_of(q.asset.as_deref())?;
    if q.source
        .as_deref()
        .is_some_and(|s| s != "claude" && s != "rules")
    {
        return Err(bad("source must be claude or rules"));
    }
    let limit = q.limit.unwrap_or(48).clamp(1, 500);
    Ok(Json(json!(app.store.judgments(
        asset.key(),
        q.source.as_deref(),
        limit
    )?)))
}

async fn ledger(State(app): State<App>, Query(q): Query<AssetQuery>) -> ApiResult {
    let asset = asset_of(q.asset.as_deref())?;
    Ok(Json(json!(judge::ledger(&app.store, asset)?)))
}

async fn brief(State(app): State<App>) -> ApiResult {
    Ok(Json(json!({
        "morning": app.store.latest_brief("morning")?,
        "weekly": app.store.latest_brief("weekly")?,
    })))
}

async fn status(State(app): State<App>) -> ApiResult {
    Ok(Json(json!({
        "claude_enabled": app.claude.is_some(),
        "telegram_enabled": app.cfg.telegram.is_some(),
        "whale_alert_enabled": app.whale_enabled(),
        "collectors": app.status.list(),
    })))
}

async fn stream(State(app): State<App>) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let events = BroadcastStream::new(app.bus.subscribe()).filter_map(|msg| {
        // A lagging client simply misses events; the next poll catches it up.
        let (name, data) = match msg.ok()? {
            BusEvent::Price { asset, ts, price } => (
                "price",
                json!({ "asset": asset.key(), "ts": ts, "price": price }),
            ),
            BusEvent::Alert(alert) => ("alert", json!(alert)),
            BusEvent::Judgment(judgment) => ("judgment", json!(judgment)),
        };
        Some(Ok(Event::default().event(name).data(data.to_string())))
    });
    Sse::new(events).keep_alive(KeepAlive::default())
}
