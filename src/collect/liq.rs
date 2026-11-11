//! Liquidation streams (Binance, Bybit, OKX) summed into per-minute USD buckets per asset and side.
//!
//! This undercounts the whole market: only three venues, and Binance pushes at most the latest
//! liquidation per symbol per second.

use std::collections::HashMap;

use anyhow::{Context, Result};
use serde_json::{Value, json};

use super::ws::{run_ws, supervise};
use crate::app::App;
use crate::db::Store;
use crate::model::Asset;
use crate::util::{floor_to, num};

pub fn spawn(app: &App) {
    supervise(app, "liq.binance", binance);
    supervise(app, "liq.bybit", bybit);
    supervise(app, "liq.okx", okx);
}

fn record(store: &Store, asset: Asset, long_liquidated: bool, usd: f64, ts_ms: i64) -> Result<()> {
    let name = if long_liquidated {
        "liq_long_usd"
    } else {
        "liq_short_usd"
    };
    store.add(&asset.m(name), floor_to(ts_ms / 1000, 60), usd)
}

fn usdt_perp(symbol: &str) -> Option<Asset> {
    Asset::parse(symbol.strip_suffix("USDT")?)
}

async fn binance(app: App) -> Result<()> {
    let store = app.store.clone();
    let url = "wss://fstream.binance.com/market/ws/!forceOrder@arr";
    run_ws(&app, "liq.binance", url, None, None, move |text| {
        let Ok(v) = serde_json::from_str::<Value>(text) else {
            return Ok(());
        };
        let o = &v["o"];
        let (Some(asset), Some(qty), Some(price), Some(ts)) = (
            o["s"].as_str().and_then(usdt_perp),
            num(&o["z"]),
            num(&o["ap"]),
            o["T"].as_i64(),
        ) else {
            return Ok(());
        };
        // The liquidation order is on the opposite side of the position: SELL closes a long.
        record(&store, asset, o["S"] == "SELL", qty * price, ts)
    })
    .await
}

async fn bybit(app: App) -> Result<()> {
    let store = app.store.clone();
    let topics: Vec<String> = Asset::ALL
        .iter()
        .map(|a| format!("allLiquidation.{}USDT", a.sym()))
        .collect();
    let subscribe = json!({ "op": "subscribe", "args": topics }).to_string();
    let url = "wss://stream.bybit.com/v5/public/linear";
    run_ws(
        &app,
        "liq.bybit",
        url,
        Some(subscribe),
        Some((20, r#"{"op":"ping"}"#)),
        move |text| {
            let Ok(v) = serde_json::from_str::<Value>(text) else {
                return Ok(());
            };
            for d in v["data"].as_array().into_iter().flatten() {
                let (Some(asset), Some(qty), Some(price), Some(ts)) = (
                    d["s"].as_str().and_then(usdt_perp),
                    num(&d["v"]),
                    num(&d["p"]),
                    d["T"].as_i64(),
                ) else {
                    continue;
                };
                // Bybit reports the position side: `Buy` means a long was liquidated.
                record(&store, asset, d["S"] == "Buy", qty * price, ts)?;
            }
            Ok(())
        },
    )
    .await
}

async fn okx(app: App) -> Result<()> {
    // OKX sizes are in contracts; the contract value comes from the instrument definition.
    let mut contracts: HashMap<String, (Asset, f64)> = HashMap::new();
    for asset in Asset::ALL {
        let inst = format!("{}-USDT-SWAP", asset.sym());
        let v = app
            .http
            .json(&format!(
                "https://www.okx.com/api/v5/public/instruments?instType=SWAP&instId={inst}"
            ))
            .await?;
        let ct_val = num(&v["data"][0]["ctVal"]).context("okx: ctVal missing")?;
        contracts.insert(inst, (asset, ct_val));
    }

    let store = app.store.clone();
    let subscribe =
        json!({ "op": "subscribe", "args": [{ "channel": "liquidation-orders", "instType": "SWAP" }] }).to_string();
    let url = "wss://ws.okx.com/ws/v5/public";
    run_ws(
        &app,
        "liq.okx",
        url,
        Some(subscribe),
        Some((25, "ping")),
        move |text| {
            // The keepalive reply is the bare string `pong`.
            let Ok(v) = serde_json::from_str::<Value>(text) else {
                return Ok(());
            };
            for row in v["data"].as_array().into_iter().flatten() {
                let Some(&(asset, ct_val)) =
                    row["instId"].as_str().and_then(|id| contracts.get(id))
                else {
                    continue;
                };
                for d in row["details"].as_array().into_iter().flatten() {
                    let (Some(size), Some(price), Some(ts)) =
                        (num(&d["sz"]), num(&d["bkPx"]), num(&d["ts"]))
                    else {
                        continue;
                    };
                    let long = match d["posSide"].as_str() {
                        Some("long") => true,
                        Some("short") => false,
                        _ => d["side"] == "sell",
                    };
                    record(&store, asset, long, size * ct_val * price, ts as i64)?;
                }
            }
            Ok(())
        },
    )
    .await
}
