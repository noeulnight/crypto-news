//! Daily net flows of US spot BTC/ETH ETFs (CoinMarketCap ETF tracker).
//!
//! The most recent session keeps being revised until every issuer has reported, so the whole
//! window is upserted on each poll.

use anyhow::{Context, Result, bail};

use crate::app::App;
use crate::util::{DAY, floor_to, num};

const URL: &str =
    "https://api.coinmarketcap.com/data-api/v3/etf/overview/netflow/chart?category=all&range=90d";

pub async fn poll(app: App) -> Result<()> {
    let v = app.http.json(URL).await?;
    let (mut btc, mut eth) = (Vec::new(), Vec::new());
    for p in v["data"]["points"]
        .as_array()
        .context("cmc: points missing")?
    {
        let Some(ts) = num(&p["timestamp"]).map(|ms| floor_to(ms as i64 / 1000, DAY)) else {
            continue;
        };
        btc.extend(num(&p["btcValue"]).map(|usd| (ts, usd)));
        eth.extend(num(&p["ethValue"]).map(|usd| (ts, usd)));
    }
    if btc.is_empty() || eth.is_empty() {
        bail!("cmc: empty ETF flow series");
    }
    app.store.put_many("etf.btc.netflow_usd", &btc)?;
    app.store.put_many("etf.eth.netflow_usd", &eth)?;
    Ok(())
}
