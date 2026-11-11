//! Cross-asset quotes (CNBC quote service) and the CME futures basis derived from them.

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Datelike, Months, NaiveDate};
use serde_json::Value;

use super::calendar::last_friday;
use crate::app::App;
use crate::model::Asset;
use crate::util::{DAY, HOUR, date_of, day_ts, floor_to, now, num};

const BARS: &str = "https://ts-api.cnbc.com/harmony/app/bars";
/// CME crypto futures stop trading at 16:00 London time on the last Friday of the month.
const EXPIRY_HOUR: i64 = 16;

const QUOTES: [(&str, &str); 7] = [
    (".VIX", "macro.vix"),
    (".DXY", "macro.dxy"),
    ("@ND.1", "macro.nq"),
    ("US2Y", "macro.us2y"),
    ("US10Y", "macro.us10y"),
    ("JPY=", "macro.usdjpy"),
    ("KRW=", "macro.usdkrw"),
];
/// Front-month CME futures.
const CME: [(&str, Asset); 2] = [("@BTC.1", Asset::Btc), ("@ETH.1", Asset::Eth)];

pub async fn poll(app: App) -> Result<()> {
    let symbols: Vec<&str> = QUOTES
        .iter()
        .map(|q| q.0)
        .chain(CME.iter().map(|c| c.0))
        .collect();
    let encoded = symbols
        .join("|")
        .replace('@', "%40")
        .replace('=', "%3D")
        .replace('|', "%7C");
    let url = format!(
        "https://quote.cnbc.com/quote-html-webservice/restQuote/symbolType/symbol?symbols={encoded}\
         &requestMethod=itv&noform=1&partnerId=2&fund=1&exthrs=1&output=json"
    );
    let v = app.http.json(&url).await?;
    let quotes = v["FormattedQuoteResult"]["FormattedQuote"]
        .as_array()
        .context("cnbc: quotes missing")?;
    let find = |symbol: &str| quotes.iter().find(|q| q["symbol"] == symbol);
    let now = now();
    let mut missing = Vec::new();

    for (symbol, metric) in QUOTES {
        match find(symbol).and_then(|q| last_print(q, now)) {
            Some((ts, last)) => app.store.put(metric, ts, last)?,
            None => missing.push(symbol),
        }
    }

    for (symbol, asset) in CME {
        let parsed = find(symbol).and_then(|q| {
            let expiry =
                NaiveDate::parse_from_str(q["expiration_date"].as_str()?, "%Y-%m-%d").ok()?;
            Some((last_print(q, now)?, expiry))
        });
        let Some(((ts, future), expiry)) = parsed else {
            missing.push(symbol);
            continue;
        };
        // The futures quote is delayed, so compare it with the USD spot price at its own print time.
        let Some((_, spot)) = app.store.at_or_before(&asset.m("cb_price"), ts + 59, 4 * 60)? else {
            continue;
        };
        if let Some(basis) = annualised_basis(future, spot, ts, day_ts(expiry) + EXPIRY_HOUR * HOUR) {
            app.store.put(&asset.m("cme_basis_annual_pct"), ts, basis)?;
        }
    }

    if missing.is_empty() {
        Ok(())
    } else {
        bail!("cnbc: no usable quote for {}", missing.join(", "))
    }
}

/// 35 days of hourly closes, so day-change z-scores and the CME basis have history on first start.
/// Must run after the spot backfill: the basis needs the Coinbase closes.
pub async fn backfill(app: &App) -> Result<()> {
    let to = now();
    let from = to - 35 * DAY;
    for (symbol, metric) in QUOTES {
        app.store.put_many(metric, &hourly_bars(app, symbol, from, to).await?)?;
    }
    for (symbol, asset) in CME {
        let mut basis = Vec::new();
        for (ts, future) in hourly_bars(app, symbol, from, to).await? {
            let (Some((_, spot)), Some(expiry)) =
                (app.store.at_or_before(&asset.m("cb_price"), ts, 4 * 60)?, front_expiry(ts))
            else {
                continue;
            };
            basis.extend(annualised_basis(future, spot, ts, expiry).map(|b| (ts, b)));
        }
        app.store.put_many(&asset.m("cme_basis_annual_pct"), &basis)?;
    }
    Ok(())
}

/// Hourly bar closes as (bar end, close), ascending.
pub async fn hourly_bars(app: &App, symbol: &str, from: i64, to: i64) -> Result<Vec<(i64, f64)>> {
    // The range is read in US Eastern time; a day of slack on the end covers the offset.
    let stamp = |ts: i64| DateTime::from_timestamp(ts, 0).unwrap_or_default().format("%Y%m%d%H%M%S").to_string();
    let symbol = symbol.replace('@', "%40").replace('=', "%3D");
    // Bars are cut relative to the range start, so it has to sit on the hour.
    let url =
        format!("{BARS}/{symbol}/1H/{}/{}/adjusted/EST5EDT.json", stamp(floor_to(from, HOUR)), stamp(to + DAY));
    let v = app.http.json(&url).await?;
    let mut bars: Vec<(i64, f64)> = v["barData"]["priceBars"]
        .as_array()
        .context("cnbc: bars missing")?
        .iter()
        .filter_map(|b| Some((num(&b["tradeTimeinMills"])? as i64 / 1000 + HOUR, num(&b["close"])?)))
        .filter(|&(ts, _)| ts <= to)
        .collect();
    bars.sort_by_key(|b| b.0);
    Ok(bars)
}

/// Expiry of the front-month contract trading at `ts`.
fn front_expiry(ts: i64) -> Option<i64> {
    let expiry = |month: NaiveDate| Some(day_ts(last_friday(month.year(), month.month())?) + EXPIRY_HOUR * HOUR);
    let month = date_of(ts).with_day(1)?;
    match expiry(month)? {
        this if ts < this => Some(this),
        _ => expiry(month.checked_add_months(Months::new(1))?),
    }
}

/// Futures premium over spot, annualised by time to expiry (percent).
fn annualised_basis(future: f64, spot: f64, ts: i64, expiry: i64) -> Option<f64> {
    let days = (expiry - ts) as f64 / DAY as f64;
    // Annualising explodes in the last days before expiry.
    (days >= 3.0).then(|| (future / spot - 1.0) * 100.0 * 365.0 / days)
}

/// (print time floored to the minute, last price). Points are keyed by print time, so a closed
/// market does not create fresh-looking samples.
fn last_print(q: &Value, now: i64) -> Option<(i64, f64)> {
    let ts = print_ts(q["last_time"].as_str()?)?;
    (ts <= now + 300 && ts >= now - 10 * DAY).then_some((floor_to(ts, 60), num(&q["last"])?))
}

/// `2026-11-11T00:55:39.000-0500`, or a bare date once the session has closed (taken as 21:00 UTC).
fn print_ts(s: &str) -> Option<i64> {
    if let Ok(dt) = DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f%z") {
        return Some(dt.timestamp());
    }
    Some(day_ts(NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()?) + 21 * HOUR)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn print_time_formats() {
        // 00:55:39 at UTC-5 is 05:55:39 UTC.
        assert_eq!(
            print_ts("2026-11-11T00:55:39.000-0500"),
            Some(1_794_376_539)
        );
        assert_eq!(
            print_ts("2026-11-10"),
            Some(day_ts(NaiveDate::from_ymd_opt(2026, 11, 10).unwrap()) + 21 * HOUR)
        );
        assert_eq!(print_ts("n/a"), None);
    }

    #[test]
    fn front_month_rolls_after_the_last_friday() {
        let at = |y, m, d, h| day_ts(NaiveDate::from_ymd_opt(y, m, d).unwrap()) + h * HOUR;
        // November 2026 expires on Friday the 27th.
        assert_eq!(front_expiry(at(2026, 11, 11, 6)), Some(at(2026, 11, 27, 16)));
        assert_eq!(front_expiry(at(2026, 11, 27, 15)), Some(at(2026, 11, 27, 16)));
        // From the expiry hour on, the next month is the front contract; December rolls into January.
        assert_eq!(front_expiry(at(2026, 11, 27, 16)), Some(at(2026, 12, 25, 16)));
        assert_eq!(front_expiry(at(2026, 12, 28, 0)), Some(at(2027, 1, 29, 16)));
    }

    #[test]
    fn basis_is_annualised_and_suppressed_near_expiry() {
        // 1% premium with 36.5 days left is 10% a year.
        let basis = annualised_basis(101.0, 100.0, 0, 365 * DAY / 10).unwrap();
        assert!((basis - 10.0).abs() < 1e-9, "{basis}");
        assert_eq!(annualised_basis(101.0, 100.0, 0, 2 * DAY), None);
    }
}
