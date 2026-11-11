//! Spot prices (Binance, Coinbase, Upbit) and the premiums derived from them.

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, NaiveDateTime, SecondsFormat};
use serde_json::Value;

use crate::app::{App, BusEvent};
use crate::model::Asset;
use super::macro_quotes::hourly_bars;
use crate::util::{DAY, HOUR, floor_to, now, num};

const BINANCE: &str = "https://api.binance.com";
const COINBASE: &str = "https://api.exchange.coinbase.com";
const UPBIT: &str = "https://api.upbit.com";

fn premium_pct(price: f64, reference: f64) -> f64 {
    (price / reference - 1.0) * 100.0
}

pub async fn poll(app: App) -> Result<()> {
    let ts = floor_to(now(), 60);
    let mut errs = Vec::new();
    let usdkrw = app
        .store
        .latest("macro.usdkrw")?
        .filter(|(t, _)| ts - t <= 4 * DAY)
        .map(|(_, v)| v);
    let upbit = match app
        .http
        .json(&format!("{UPBIT}/v1/ticker?markets=KRW-BTC,KRW-ETH"))
        .await
    {
        Ok(v) => Some(v),
        Err(e) => {
            errs.push(format!("upbit: {e:#}"));
            None
        }
    };

    for asset in Asset::ALL {
        let sym = asset.sym();
        let bn = app
            .http
            .json(&format!("{BINANCE}/api/v3/ticker/price?symbol={sym}USDT"))
            .await?;
        let price = num(&bn["price"]).context("binance: price missing")?;
        app.store.put(&asset.m("price"), ts, price)?;
        let _ = app.bus.send(BusEvent::Price { asset, ts, price });

        match app
            .http
            .json(&format!("{COINBASE}/products/{sym}-USD/ticker"))
            .await
        {
            Ok(cb) => match num(&cb["price"]) {
                Some(usd) => {
                    app.store.put(&asset.m("cb_price"), ts, usd)?;
                    app.store.put(
                        &asset.m("coinbase_premium_pct"),
                        ts,
                        premium_pct(usd, price),
                    )?;
                }
                None => errs.push(format!("coinbase {sym}: price missing")),
            },
            Err(e) => errs.push(format!("coinbase {sym}: {e:#}")),
        }

        if let (Some(u), Some(fx)) = (&upbit, usdkrw) {
            let market = format!("KRW-{sym}");
            let krw = u
                .as_array()
                .and_then(|rows| rows.iter().find(|r| r["market"] == market.as_str()))
                .and_then(|r| num(&r["trade_price"]));
            match krw {
                Some(krw) => app.store.put(
                    &asset.m("kimchi_premium_pct"),
                    ts,
                    premium_pct(krw, price * fx),
                )?,
                None => errs.push(format!("upbit {market}: price missing")),
            }
        }
    }
    if errs.is_empty() {
        Ok(())
    } else {
        bail!(errs.join("; "))
    }
}

/// Hourly closes for the last ~35 days: price, Coinbase premium and kimchi premium.
pub async fn backfill(app: &App) -> Result<()> {
    let end = floor_to(now(), HOUR);
    let start = end - 35 * DAY;
    // Hourly USD/KRW; weekends fall back to the last print.
    let fx = hourly_bars(app, "KRW=", start - 4 * DAY, end).await?;

    for asset in Asset::ALL {
        let sym = asset.sym();

        let klines = app
            .http
            .json(&format!(
                "{BINANCE}/api/v3/klines?symbol={sym}USDT&interval=1h&limit=1000"
            ))
            .await?;
        let binance: Vec<(i64, f64)> = klines
            .as_array()
            .context("binance klines: not an array")?
            .iter()
            .filter_map(|k| Some((k[0].as_i64()? / 1000 + HOUR, num(&k[4])?)))
            .filter(|&(ts, _)| ts <= end)
            .collect();
        app.store.put_many(&asset.m("price"), &binance)?;
        let by_ts: HashMap<i64, f64> = binance.iter().copied().collect();

        // Coinbase returns at most 300 candles per request.
        let mut coinbase = Vec::new();
        let mut to = end;
        while to > start {
            let from = (to - 300 * HOUR).max(start);
            let url = format!(
                "{COINBASE}/products/{sym}-USD/candles?granularity=3600&start={}&end={}",
                iso(from),
                iso(to)
            );
            let rows = app.http.json(&url).await?;
            coinbase.extend(
                rows.as_array()
                    .context("coinbase candles: not an array")?
                    .iter()
                    .filter_map(|c| Some((c[0].as_i64()? + HOUR, num(&c[4])?)))
                    .filter(|&(ts, _)| ts <= end),
            );
            to = from;
        }
        app.store.put_many(&asset.m("cb_price"), &coinbase)?;
        let premium: Vec<(i64, f64)> = coinbase
            .iter()
            .filter_map(|&(ts, usd)| Some((ts, premium_pct(usd, *by_ts.get(&ts)?))))
            .collect();
        app.store
            .put_many(&asset.m("coinbase_premium_pct"), &premium)?;

        // Upbit returns at most 200 candles per request, newest first.
        let mut kimchi = Vec::new();
        let mut to = end;
        while to > start {
            let url = format!(
                "{UPBIT}/v1/candles/minutes/60?market=KRW-{sym}&count=200&to={}",
                iso(to)
            );
            let rows = app.http.json(&url).await?;
            let rows = rows.as_array().context("upbit candles: not an array")?;
            let mut oldest = to;
            for c in rows {
                let (Some(open), Some(krw)) =
                    (upbit_ts(&c["candle_date_time_utc"]), num(&c["trade_price"]))
                else {
                    continue;
                };
                oldest = oldest.min(open);
                let ts = open + HOUR;
                if let (Some(usdt), Some(rate)) = (by_ts.get(&ts), rate_at(&fx, ts)) {
                    kimchi.push((ts, premium_pct(krw, usdt * rate)));
                }
            }
            if rows.is_empty() || oldest >= to {
                break;
            }
            to = oldest;
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        app.store
            .put_many(&asset.m("kimchi_premium_pct"), &kimchi)?;
    }
    Ok(())
}

fn iso(ts: i64) -> String {
    DateTime::from_timestamp(ts, 0)
        .unwrap_or_default()
        .to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn upbit_ts(v: &Value) -> Option<i64> {
    let dt = NaiveDateTime::parse_from_str(v.as_str()?, "%Y-%m-%dT%H:%M:%S").ok()?;
    Some(dt.and_utc().timestamp())
}

fn rate_at(rates: &[(i64, f64)], ts: i64) -> Option<f64> {
    let idx = rates.partition_point(|r| r.0 <= ts);
    rates.get(idx.checked_sub(1)?).map(|r| r.1)
}
