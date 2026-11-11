//! Market-implied odds for the next FOMC decision (Kalshi `KXFED` ladder) and FOMC dates.
//!
//! Each Kalshi market resolves Yes when the upper bound of the target range ends above its strike,
//! so the ladder is a survival curve over the post-meeting rate.

use anyhow::{Context, Result};
use chrono::DateTime;
use serde_json::Value;

use crate::app::App;
use crate::model::Event;
use crate::util::{floor_to, now, num};

const KALSHI: &str = "https://api.elections.kalshi.com/trade-api/v2";
const EFFR: &str = "https://markets.newyorkfed.org/api/rates/unsecured/effr/last/1.json";

#[derive(Debug)]
pub struct Odds {
    pub p_cut: f64,
    pub p_hold: f64,
    pub p_hike: f64,
    /// Expected change of the upper bound at the meeting, in basis points.
    pub expected_bp: f64,
}

/// `upper`: current upper bound of the target range. `ladder`: (strike, P(upper bound > strike)).
pub fn odds(upper: f64, ladder: &[(f64, f64)]) -> Option<Odds> {
    let mut ladder = ladder.to_vec();
    ladder.sort_by(|a, b| a.0.total_cmp(&b.0));
    // Tail quotes are noisy; a survival curve cannot rise with the strike.
    for i in 1..ladder.len() {
        ladder[i].1 = ladder[i].1.min(ladder[i - 1].1);
    }
    let above = |strike: f64| {
        ladder
            .iter()
            .find(|p| (p.0 - strike).abs() < 1e-6)
            .map(|p| p.1)
    };
    let p_hike = above(upper)?;
    let p_not_cut = above(upper - 0.25)?;

    // Outcomes sit on the 25bp grid: P(rate == s[i+1]) = P(> s[i]) - P(> s[i+1]).
    let (first, last) = (ladder.first()?, ladder.last()?);
    let mut expected = first.0 * (1.0 - first.1) + (last.0 + 0.25) * last.1;
    for w in ladder.windows(2) {
        expected += w[1].0 * (w[0].1 - w[1].1);
    }
    Some(Odds {
        p_cut: (1.0 - p_not_cut) * 100.0,
        p_hold: (p_not_cut - p_hike) * 100.0,
        p_hike: p_hike * 100.0,
        expected_bp: (expected - upper) * 100.0,
    })
}

fn yes_probability(market: &Value) -> Option<f64> {
    match (
        num(&market["yes_bid_dollars"]),
        num(&market["yes_ask_dollars"]),
    ) {
        (Some(bid), Some(ask)) if ask > 0.0 => Some((bid + ask) / 2.0),
        _ => num(&market["last_price_dollars"]),
    }
}

pub async fn poll(app: App) -> Result<()> {
    let effr = app.http.json(EFFR).await?;
    let upper = num(&effr["refRates"][0]["targetRateTo"]).context("nyfed: target range missing")?;

    let url = format!(
        "{KALSHI}/events?series_ticker=KXFED&status=open&with_nested_markets=true&limit=20"
    );
    let v = app.http.json(&url).await?;
    let now = now();
    let mut next: Option<(i64, &Value)> = None;
    for event in v["events"].as_array().context("kalshi: events missing")? {
        let (Some(ticker), Some(ts)) = (
            event["event_ticker"].as_str(),
            event["strike_date"]
                .as_str()
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|d| d.timestamp()),
        ) else {
            continue;
        };
        app.store.upsert_event(&Event {
            id: format!("fomc:{ticker}"),
            ts,
            title: "FOMC 금리 결정".into(),
            kind: "fomc".into(),
            impact: "high".into(),
            forecast: None,
            previous: None,
        })?;
        if ts > now && next.is_none_or(|(t, _)| ts < t) {
            next = Some((ts, event));
        }
    }

    let (_, event) = next.context("kalshi: no upcoming meeting")?;
    let ladder: Vec<(f64, f64)> = event["markets"]
        .as_array()
        .context("kalshi: markets missing")?
        .iter()
        .filter_map(|m| Some((num(&m["floor_strike"])?, yes_probability(m)?)))
        .collect();
    let o =
        odds(upper, &ladder).context("kalshi: ladder does not cover the current target rate")?;

    let ts = floor_to(now, 600);
    app.store.put("macro.fed.p_cut", ts, o.p_cut)?;
    app.store.put("macro.fed.p_hold", ts, o.p_hold)?;
    app.store.put("macro.fed.p_hike", ts, o.p_hike)?;
    app.store.put("macro.fed.expected_bp", ts, o.expected_bp)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn odds_from_a_hike_priced_ladder() {
        // Mid quotes observed for a meeting with the target range at 3.75-4.00.
        let ladder = [
            (2.75, 0.99),
            (3.0, 0.995),
            (3.25, 0.995),
            (3.5, 0.995),
            (3.75, 0.995),
            (4.0, 0.955),
            (4.25, 0.105),
            (4.5, 0.015),
            (4.75, 0.005),
            (5.0, 0.005),
            (5.25, 0.005),
        ];
        let o = odds(4.0, &ladder).unwrap();
        assert!(close(o.p_hike, 95.5), "{o:?}");
        // The 0.995 quotes above the 2.75 strike are capped at 0.99 by monotonicity.
        assert!(close(o.p_cut, 1.0), "{o:?}");
        assert!(close(o.p_hold, 3.5), "{o:?}");
        assert!(close(o.p_cut + o.p_hold + o.p_hike, 100.0), "{o:?}");
        assert!(close(o.expected_bp, 26.0), "{o:?}");
    }

    #[test]
    fn odds_need_strikes_at_and_just_below_the_current_rate() {
        assert!(odds(4.0, &[(4.25, 0.1), (4.5, 0.01)]).is_none());
    }
}
