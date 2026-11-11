//! Data collectors. Each one owns one external source and writes into the store.

pub mod calendar;
pub mod derivs;
pub mod etf;
pub mod fed;
pub mod liquidity;
pub mod macro_quotes;
pub mod news;
pub mod onchain;
pub mod options;
pub mod spot;
pub mod stables;

use std::time::Duration;

use anyhow::Result;
use tokio::time::{MissedTickBehavior, interval};

use crate::app::App;
use crate::util::now;

/// Runs `f` immediately and then every `secs`; failures are recorded and never stop the loop.
pub fn every<F, Fut>(app: &App, name: &'static str, secs: u64, f: F)
where
    F: Fn(App) -> Fut + Send + 'static,
    Fut: Future<Output = Result<()>> + Send + 'static,
{
    let app = app.clone();
    app.status.expect(name, secs);
    tokio::spawn(async move {
        // Tokio timers follow the monotonic clock, which stops while a Mac sleeps. Checking the
        // wall clock on a short tick runs an overdue job right after wake, not a full period later.
        let mut tick = interval(Duration::from_secs(secs.min(15)));
        tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut due = 0;
        loop {
            tick.tick().await;
            let t = now();
            // One second of slack, since `now` has whole-second resolution.
            if t + 1 < due {
                continue;
            }
            due = t + secs as i64;
            match f(app.clone()).await {
                Ok(()) => app.status.ok(name),
                Err(e) => {
                    tracing::warn!(collector = name, "{e:#}");
                    app.status.err(name, format!("{e:#}"));
                }
            }
        }
    });
}

pub fn spawn_all(app: &App) {
    every(app, "spot", 60, spot::poll);
    every(app, "macro", 300, macro_quotes::poll);
    every(app, "derivs", 300, derivs::poll);
    every(app, "options", 600, options::poll);
    every(app, "fed", 1800, fed::poll);
    every(app, "etf", 1800, etf::poll);
    every(app, "stables", 3600, stables::poll);
    every(app, "onchain", 3600, onchain::poll);
    every(app, "liquidity", 6 * 3600, liquidity::poll);
    every(app, "calendar", 3 * 3600, calendar::poll);
    every(app, "news", 60, news::poll);
    if app.claude.is_some() {
        every(app, "news.tag", 120, news::tag);
    }
}

/// One-off history load so that 30-day z-scores are meaningful on first start. Idempotent.
pub async fn backfill(app: &App) {
    // Spot first: the CME basis history in the macro backfill is computed against its closes.
    let spot = spot::backfill(app).await;
    let (derivs, options, liquidity, macro_quotes) = tokio::join!(
        derivs::backfill(app),
        options::backfill(app),
        liquidity::backfill(app),
        macro_quotes::backfill(app),
    );
    for (name, res) in [
        ("spot", spot),
        ("derivs", derivs),
        ("options", options),
        ("liquidity", liquidity),
        ("macro", macro_quotes),
    ] {
        match res {
            Ok(()) => tracing::info!("backfill {name}: done"),
            Err(e) => tracing::warn!("backfill {name}: {e:#}"),
        }
    }
}
