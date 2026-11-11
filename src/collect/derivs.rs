//! Perpetual funding, open interest and quarterly futures basis.

use std::collections::HashMap;

use anyhow::{Context, Result};
use serde_json::Value;

use crate::app::App;
use crate::model::Asset;
use crate::util::{HOUR, floor_to, now, num};

const BINANCE: &str = "https://fapi.binance.com";
const BYBIT: &str = "https://api.bybit.com";

pub async fn poll(app: App) -> Result<()> {
    let ts = floor_to(now(), 300);
    for asset in Asset::ALL {
        let sym = format!("{}USDT", asset.sym());

        let premium = app
            .http
            .json(&format!("{BINANCE}/fapi/v1/premiumIndex?symbol={sym}"))
            .await?;
        let mark = num(&premium["markPrice"]).context("binance: markPrice missing")?;
        let funding_binance =
            num(&premium["lastFundingRate"]).context("binance: funding missing")?;

        let oi = app
            .http
            .json(&format!("{BINANCE}/fapi/v1/openInterest?symbol={sym}"))
            .await?;
        let contracts = num(&oi["openInterest"]).context("binance: openInterest missing")?;
        app.store.put(&asset.m("oi_usd"), ts, contracts * mark)?;

        let bybit = app
            .http
            .json(&format!(
                "{BYBIT}/v5/market/tickers?category=linear&symbol={sym}"
            ))
            .await?;
        let funding_bybit =
            num(&bybit["result"]["list"][0]["fundingRate"]).context("bybit: funding missing")?;
        app.store.put(
            &asset.m("funding_avg"),
            ts,
            (funding_binance + funding_bybit) / 2.0 * 100.0,
        )?;

        let basis = app
            .http
            .json(&format!("{BINANCE}/futures/data/basis?pair={sym}&contractType=CURRENT_QUARTER&period=5m&limit=1"))
            .await?;
        // The annualised rate is blank in the last days before delivery.
        if let Some(rate) = basis
            .as_array()
            .and_then(|rows| rows.last())
            .and_then(|r| num(&r["annualizedBasisRate"]))
        {
            app.store
                .put(&asset.m("basis_q_annual_pct"), ts, rate * 100.0)?;
        }
    }
    Ok(())
}

pub async fn backfill(app: &App) -> Result<()> {
    for asset in Asset::ALL {
        let sym = format!("{}USDT", asset.sym());

        // Settled funding every 8h; averaged where both venues have the same settlement time.
        let binance = app
            .http
            .json(&format!(
                "{BINANCE}/fapi/v1/fundingRate?symbol={sym}&limit=1000"
            ))
            .await?;
        let binance: HashMap<i64, f64> = rows(&binance)?
            .iter()
            .filter_map(|r| {
                Some((
                    floor_to(r["fundingTime"].as_i64()? / 1000, HOUR),
                    num(&r["fundingRate"])?,
                ))
            })
            .collect();
        let bybit = app
            .http
            .json(&format!(
                "{BYBIT}/v5/market/funding/history?category=linear&symbol={sym}&limit=200"
            ))
            .await?;
        let funding: Vec<(i64, f64)> = rows(&bybit["result"]["list"])?
            .iter()
            .filter_map(|r| {
                let ts = floor_to(num(&r["fundingRateTimestamp"])? as i64 / 1000, HOUR);
                Some((
                    ts,
                    (num(&r["fundingRate"])? + binance.get(&ts)?) / 2.0 * 100.0,
                ))
            })
            .collect();
        app.store.put_many(&asset.m("funding_avg"), &funding)?;

        let oi = paged(
            app,
            &format!("{BINANCE}/futures/data/openInterestHist?symbol={sym}&period=1h&limit=500"),
        )
        .await?;
        let oi: Vec<(i64, f64)> = oi
            .iter()
            .filter_map(|r| {
                Some((
                    r["timestamp"].as_i64()? / 1000,
                    num(&r["sumOpenInterestValue"])?,
                ))
            })
            .collect();
        app.store.put_many(&asset.m("oi_usd"), &oi)?;

        let basis = paged(
            app,
            &format!("{BINANCE}/futures/data/basis?pair={sym}&contractType=CURRENT_QUARTER&period=1h&limit=500"),
        )
        .await?;
        let basis: Vec<(i64, f64)> = basis
            .iter()
            .filter_map(|r| {
                Some((
                    r["timestamp"].as_i64()? / 1000,
                    num(&r["annualizedBasisRate"])? * 100.0,
                ))
            })
            .collect();
        app.store.put_many(&asset.m("basis_q_annual_pct"), &basis)?;
    }
    Ok(())
}

fn rows(v: &Value) -> Result<&Vec<Value>> {
    v.as_array().context("expected a JSON array")
}

/// Binance keeps about 30 days of these series, 500 rows per request: fetch two pages.
async fn paged(app: &App, url: &str) -> Result<Vec<Value>> {
    let mut all = rows(&app.http.json(url).await?)?.clone();
    if let Some(oldest) = all.iter().filter_map(|r| r["timestamp"].as_i64()).min() {
        // The older page is best-effort: Binance rejects ranges beyond its retention window.
        if let Ok(page) = app
            .http
            .json(&format!("{url}&endTime={}", oldest - 1))
            .await
        {
            all.extend(rows(&page)?.iter().cloned());
        }
    }
    Ok(all)
}
