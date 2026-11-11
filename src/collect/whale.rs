//! Large BTC/ETH transfers into exchanges (Whale Alert Alerts API, paid key required).
//!
//! Only transfers whose receiver is a known exchange are kept; everything else is noise for a
//! short-term sell-pressure signal.

use anyhow::{Context, Result};
use serde_json::{Value, json};

use super::ws::{run_ws, supervise};
use crate::app::App;
use crate::model::Asset;
use crate::notify::{self, Draft};
use crate::util::{floor_to, grouped, num, usd};

const MIN_USD: f64 = 20_000_000.0;
const EXCHANGES: [&str; 19] = [
    "binance",
    "coinbase",
    "kraken",
    "okx",
    "okex",
    "bybit",
    "bitfinex",
    "upbit",
    "bithumb",
    "kucoin",
    "gemini",
    "bitstamp",
    "htx",
    "huobi",
    "gate",
    "crypto.com",
    "mexc",
    "deribit",
    "bitget",
];

#[derive(Debug, PartialEq)]
pub struct Inflow {
    pub ts: i64,
    pub asset: Asset,
    pub amount: f64,
    pub usd: f64,
    pub from: String,
    pub to: String,
}

/// BTC/ETH amounts of an alert that moved from outside an exchange into it.
pub fn exchange_inflows(alert: &Value) -> Vec<Inflow> {
    if alert["transaction_type"] != "transfer" {
        return Vec::new();
    }
    let (Some(ts), Some(from), Some(to)) = (
        alert["timestamp"].as_i64(),
        alert["from"].as_str(),
        alert["to"].as_str(),
    ) else {
        return Vec::new();
    };
    let (from_lc, to_lc) = (from.to_lowercase(), to.to_lowercase());
    let Some(exchange) = EXCHANGES.iter().find(|e| to_lc.contains(**e)) else {
        return Vec::new();
    };
    // Same owner on both sides is an internal wallet shuffle.
    if from_lc.contains(exchange) {
        return Vec::new();
    }
    alert["amounts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| {
            Some(Inflow {
                ts,
                asset: Asset::parse(a["symbol"].as_str()?)?,
                amount: num(&a["amount"])?,
                usd: num(&a["value_usd"])?,
                from: from.into(),
                to: to.into(),
            })
        })
        .collect()
}

pub fn spawn(app: &App) {
    if app.whale_enabled() {
        supervise(app, "whale", run);
    }
}

async fn run(app: App) -> Result<()> {
    let key = app
        .cfg
        .whale_alert_key
        .as_deref()
        .context("WHALE_ALERT_API_KEY not set")?;
    let url = format!("wss://leviathan.whale-alert.io/ws?api_key={key}");
    let subscribe = json!({
        "type": "subscribe_alerts",
        "id": "crypto-news",
        "symbols": ["btc", "eth"],
        "tx_types": ["transfer"],
        "min_value_usd": MIN_USD,
    })
    .to_string();
    let sink = app.clone();
    run_ws(&app, "whale", &url, Some(subscribe), None, move |text| {
        let Ok(v) = serde_json::from_str::<Value>(text) else {
            return Ok(());
        };
        for t in exchange_inflows(&v) {
            sink.store
                .add(&t.asset.m("whale_to_ex_usd"), floor_to(t.ts, 60), t.usd)?;
            notify::emit(
                &sink,
                Draft {
                    tier: "realtime",
                    severity: if t.usd >= 100e6 { "warn" } else { "info" },
                    rule: "whale_to_exchange",
                    asset: Some(t.asset),
                    title: format!("{} 고래 입금, {}", t.asset.sym(), t.to),
                    body: format!(
                        "{} {} ({}), {}에서 {}로",
                        grouped(t.amount, 0),
                        t.asset.sym(),
                        usd(t.usd),
                        t.from,
                        t.to
                    ),
                    dedupe: None,
                    cooldown: 0,
                },
            )?;
        }
        Ok(())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alert(from: &str, to: &str, tx_type: &str) -> Value {
        // Shape of the alert example in the Whale Alert Alerts API documentation.
        json!({
            "channel_id": "xlLZ7tJq",
            "timestamp": 1687389431,
            "blockchain": "bitcoin",
            "transaction_type": tx_type,
            "from": from,
            "to": to,
            "amounts": [
                { "symbol": "BTC", "amount": 1500.5, "value_usd": 110000000.0 },
                { "symbol": "USDC", "amount": 20006425.31, "value_usd": 20008425.95 }
            ],
            "text": "…"
        })
    }

    #[test]
    fn keeps_only_btc_eth_moving_into_an_exchange() {
        let inflows = exchange_inflows(&alert("unknown wallet", "Binance", "transfer"));
        assert_eq!(
            inflows,
            [Inflow {
                ts: 1687389431,
                asset: Asset::Btc,
                amount: 1500.5,
                usd: 110000000.0,
                from: "unknown wallet".into(),
                to: "Binance".into()
            }]
        );
    }

    #[test]
    fn drops_outflows_internal_moves_and_non_transfers() {
        assert!(exchange_inflows(&alert("Binance", "unknown wallet", "transfer")).is_empty());
        assert!(exchange_inflows(&alert("Binance", "Binance", "transfer")).is_empty());
        assert!(
            exchange_inflows(&alert("unknown wallet", "unknown wallet", "transfer")).is_empty()
        );
        assert!(exchange_inflows(&alert("Tether Treasury", "Binance", "mint")).is_empty());
    }
}
