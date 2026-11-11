//! Exchange flows: Coin Metrics community data (daily, per asset) and DefiLlama CEX inflows (all assets).

use anyhow::{Context, Result, bail};
use chrono::DateTime;

use crate::app::App;
use crate::model::Asset;
use crate::util::{DAY, HOUR, date_of, floor_to, now, num};

pub async fn poll(app: App) -> Result<()> {
    let start = date_of(now() - 100 * DAY);
    let url = format!(
        "https://community-api.coinmetrics.io/v4/timeseries/asset-metrics?assets=btc,eth\
         &metrics=FlowInExNtv,FlowOutExNtv,SplyExNtv&frequency=1d&start_time={start}&page_size=1000"
    );
    let v = app.http.json(&url).await?;
    let rows = v["data"].as_array().context("coinmetrics: data missing")?;
    for asset in Asset::ALL {
        let (mut netflow, mut supply) = (Vec::new(), Vec::new());
        for row in rows.iter().filter(|r| r["asset"] == asset.key()) {
            let Some(ts) = row["time"]
                .as_str()
                .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
            else {
                continue;
            };
            let ts = floor_to(ts.timestamp(), DAY);
            if let (Some(inflow), Some(outflow)) =
                (num(&row["FlowInExNtv"]), num(&row["FlowOutExNtv"]))
            {
                netflow.push((ts, inflow - outflow));
            }
            supply.extend(num(&row["SplyExNtv"]).map(|coins| (ts, coins)));
        }
        if netflow.is_empty() {
            bail!("coinmetrics: no exchange flow rows for {}", asset.key());
        }
        app.store.put_many(&asset.m("ex_netflow"), &netflow)?;
        app.store.put_many(&asset.m("ex_supply"), &supply)?;
    }

    let v = app.http.json("https://api.llama.fi/cexs").await?;
    let total: f64 = v["cexs"]
        .as_array()
        .context("defillama: cexs missing")?
        .iter()
        .filter_map(|c| num(&c["inflows_24h"]))
        .sum();
    app.store
        .put("cex.inflow_24h_usd", floor_to(now(), HOUR), total)?;
    Ok(())
}
