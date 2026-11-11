//! USDT + USDC circulating supply (DefiLlama) and net issuance over 1 and 7 days.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};

use crate::app::App;
use crate::util::{DAY, floor_to, num};

/// DefiLlama stablecoin ids: 1 = USDT, 2 = USDC.
const IDS: [u32; 2] = [1, 2];
const DAYS: usize = 140;

pub async fn poll(app: App) -> Result<()> {
    let mut by_day: BTreeMap<i64, (f64, usize)> = BTreeMap::new();
    for id in IDS {
        let v = app
            .http
            .json(&format!(
                "https://stablecoins.llama.fi/stablecoincharts/all?stablecoin={id}"
            ))
            .await?;
        for row in v
            .as_array()
            .context("defillama: chart is not an array")?
            .iter()
            .rev()
            .take(DAYS)
        {
            let (Some(ts), Some(supply)) = (
                num(&row["date"]),
                num(&row["totalCirculating"]["peggedUSD"]),
            ) else {
                continue;
            };
            let day = by_day.entry(floor_to(ts as i64, DAY)).or_default();
            day.0 += supply;
            day.1 += 1;
        }
    }
    // Only days where both coins reported, so the sum is comparable day to day.
    let supply: Vec<(i64, f64)> = by_day
        .into_iter()
        .filter(|(_, (_, n))| *n == IDS.len())
        .map(|(ts, (usd, _))| (ts, usd))
        .collect();
    if supply.len() < 8 {
        bail!("defillama: only {} usable days", supply.len());
    }
    let net = |days: i64| -> Vec<(i64, f64)> {
        supply
            .iter()
            .filter_map(|&(ts, usd)| {
                let prev = supply
                    .binary_search_by_key(&(ts - days * DAY), |p| p.0)
                    .ok()?;
                Some((ts, usd - supply[prev].1))
            })
            .collect()
    };
    app.store.put_many("stable.supply_usd", &supply)?;
    app.store.put_many("stable.net_1d_usd", &net(1))?;
    app.store.put_many("stable.net_7d_usd", &net(7))?;
    Ok(())
}
