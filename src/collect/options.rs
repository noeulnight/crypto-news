//! Deribit implied volatility index (DVOL) and 25-delta skew.

use std::f64::consts::SQRT_2;

use anyhow::{Context, Result};
use chrono::NaiveDate;
use serde_json::Value;

use crate::app::App;
use crate::model::Asset;
use crate::util::{DAY, HOUR, day_ts, floor_to, now, num};

const DERIBIT: &str = "https://www.deribit.com/api/v2/public";
const YEAR: f64 = 365.0 * DAY as f64;

pub async fn poll(app: App) -> Result<()> {
    let now = now();
    for asset in Asset::ALL {
        let candles = dvol(&app, asset, now - 900, now, 60).await?;
        let &(ts, close) = candles.last().context("deribit: no DVOL data")?;
        app.store.put(&asset.m("dvol"), ts, close)?;

        let url = format!(
            "{DERIBIT}/get_book_summary_by_currency?currency={}&kind=option",
            asset.sym()
        );
        let book = app.http.json(&url).await?;
        let quotes: Vec<Quote> = book["result"]
            .as_array()
            .context("deribit: option book missing")?
            .iter()
            .filter_map(Quote::parse)
            .collect();
        if let Some(skew) = skew_25d(&quotes, now) {
            app.store
                .put(&asset.m("skew25d"), floor_to(now, 600), skew)?;
        }
    }
    Ok(())
}

pub async fn backfill(app: &App) -> Result<()> {
    let now = now();
    for asset in Asset::ALL {
        let candles = dvol(app, asset, now - 40 * DAY, now, HOUR).await?;
        app.store.put_many(&asset.m("dvol"), &candles)?;
    }
    Ok(())
}

/// DVOL candle closes as (close time, value).
async fn dvol(
    app: &App,
    asset: Asset,
    from: i64,
    to: i64,
    resolution: i64,
) -> Result<Vec<(i64, f64)>> {
    let url = format!(
        "{DERIBIT}/get_volatility_index_data?currency={}&start_timestamp={}&end_timestamp={}&resolution={resolution}",
        asset.sym(),
        from * 1000,
        to * 1000
    );
    let v = app.http.json(&url).await?;
    Ok(v["result"]["data"]
        .as_array()
        .context("deribit: DVOL data missing")?
        .iter()
        .filter_map(|c| Some((c[0].as_i64()? / 1000 + resolution, num(&c[4])?)))
        .filter(|&(ts, _)| ts <= to)
        .collect())
}

struct Quote {
    expiry: i64,
    strike: f64,
    call: bool,
    /// Mark implied volatility in percent.
    iv: f64,
    forward: f64,
}

impl Quote {
    fn parse(v: &Value) -> Option<Self> {
        let (expiry, strike, call) = parse_instrument(v["instrument_name"].as_str()?)?;
        let iv = num(&v["mark_iv"]).filter(|iv| *iv > 0.0)?;
        Some(Self {
            expiry,
            strike,
            call,
            iv,
            forward: num(&v["underlying_price"])?,
        })
    }
}

/// `BTC-25DEC26-106000-P` -> (expiry at 08:00 UTC, strike, is_call).
fn parse_instrument(name: &str) -> Option<(i64, f64, bool)> {
    let mut parts = name.split('-').skip(1);
    let date = NaiveDate::parse_from_str(parts.next()?, "%d%b%y").ok()?;
    let strike = parts.next()?.parse().ok()?;
    let call = match parts.next()? {
        "C" => true,
        "P" => false,
        _ => return None,
    };
    Some((day_ts(date) + 8 * HOUR, strike, call))
}

fn erf(x: f64) -> f64 {
    // Abramowitz & Stegun 7.1.26, absolute error below 1.5e-7.
    let t = 1.0 / (1.0 + 0.327_591_1 * x.abs());
    let poly = ((((1.061_405_429 * t - 1.453_152_027) * t + 1.421_413_741) * t - 0.284_496_736)
        * t
        + 0.254_829_592)
        * t;
    let y = 1.0 - poly * (-x * x).exp();
    if x >= 0.0 { y } else { -y }
}

fn norm_cdf(x: f64) -> f64 {
    0.5 * (1.0 + erf(x / SQRT_2))
}

/// Linear interpolation of IV at `target` delta; `None` when the chain does not bracket it.
fn iv_at(mut points: Vec<(f64, f64)>, target: f64) -> Option<f64> {
    points.sort_by(|a, b| a.0.total_cmp(&b.0));
    points
        .windows(2)
        .find(|w| w[0].0 <= target && target <= w[1].0)
        .map(|w| {
            let span = w[1].0 - w[0].0;
            if span <= f64::EPSILON {
                w[0].1
            } else {
                w[0].1 + (w[1].1 - w[0].1) * (target - w[0].0) / span
            }
        })
}

/// IV(25-delta put) - IV(25-delta call) in vol points, on the expiry closest to 30 days out.
/// Positive means downside protection is bid.
fn skew_25d(quotes: &[Quote], now: i64) -> Option<f64> {
    let days = |expiry: i64| (expiry - now) as f64 / DAY as f64;
    let expiry = quotes
        .iter()
        .map(|q| q.expiry)
        .filter(|&e| (7.0..=90.0).contains(&days(e)))
        .min_by(|a, b| (days(*a) - 30.0).abs().total_cmp(&(days(*b) - 30.0).abs()))?;
    let t = (expiry - now) as f64 / YEAR;

    let (mut calls, mut puts) = (Vec::new(), Vec::new());
    for q in quotes.iter().filter(|q| q.expiry == expiry) {
        let sigma = q.iv / 100.0;
        let d1 = ((q.forward / q.strike).ln() + 0.5 * sigma * sigma * t) / (sigma * t.sqrt());
        // Forward delta with zero rates.
        if q.call {
            calls.push((norm_cdf(d1), q.iv))
        } else {
            puts.push((norm_cdf(d1) - 1.0, q.iv))
        }
    }
    Some(iv_at(puts, -0.25)? - iv_at(calls, 0.25)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instrument_names_with_one_and_two_digit_days() {
        let (expiry, strike, call) = parse_instrument("BTC-25DEC26-106000-P").unwrap();
        assert_eq!(
            expiry,
            day_ts(NaiveDate::from_ymd_opt(2026, 12, 25).unwrap()) + 8 * HOUR
        );
        assert_eq!((strike, call), (106_000.0, false));
        let (expiry, _, call) = parse_instrument("ETH-5DEC26-2000-C").unwrap();
        assert_eq!(
            expiry,
            day_ts(NaiveDate::from_ymd_opt(2026, 12, 5).unwrap()) + 8 * HOUR
        );
        assert!(call);
        assert!(parse_instrument("BTC-PERPETUAL").is_none());
    }

    fn chain(now: i64, expiry: i64, iv: impl Fn(f64) -> f64) -> Vec<Quote> {
        let forward = 100.0;
        let _ = now;
        (60..=140)
            .step_by(5)
            .flat_map(|k| {
                let strike = k as f64;
                [true, false].map(|call| Quote {
                    expiry,
                    strike,
                    call,
                    iv: iv(strike),
                    forward,
                })
            })
            .collect()
    }

    #[test]
    fn skew_is_zero_on_flat_smile_and_positive_when_puts_are_richer() {
        let now = 1_700_000_000;
        let expiry = now + 30 * DAY;
        let flat = skew_25d(&chain(now, expiry, |_| 50.0), now).unwrap();
        assert!(flat.abs() < 1e-9, "flat smile skew = {flat}");

        // IV rises as strikes fall: 25-delta put (low strike) must carry more IV than the 25-delta call.
        let skewed = skew_25d(&chain(now, expiry, |k| 50.0 + (100.0 - k) * 0.2), now).unwrap();
        assert!(skewed > 1.0, "downside skew = {skewed}");
    }

    #[test]
    fn skew_ignores_expiries_outside_the_one_week_to_three_month_band() {
        let now = 1_700_000_000;
        assert!(skew_25d(&chain(now, now + 2 * DAY, |_| 50.0), now).is_none());
    }
}
