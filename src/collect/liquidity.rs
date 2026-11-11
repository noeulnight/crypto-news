//! Fed net liquidity = balance sheet (H.4.1 total assets) - Treasury General Account - reverse repo.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use anyhow::{Context, Result};
use chrono::{Days, NaiveDate};
use regex::Regex;

use crate::app::App;
use crate::util::{DAY, date_of, day_ts, now, num, parse_num};

const H41: &str = "https://www.federalreserve.gov/releases/h41";
const TGA: &str = "https://api.fiscaldata.treasury.gov/services/api/fiscal_service/v1/accounting/dts/\
     operating_cash_balance?fields=record_date,account_type,open_today_bal&sort=-record_date&page%5Bsize%5D=600";
const TGA_CLOSING: &str = "Treasury General Account (TGA) Closing Balance";
const RRP: &str = "https://markets.newyorkfed.org/api/rp/reverserepo/propositions/search.json";
const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];

// Tags and character references (the tables pad numbers with `&#xa0;`).
static MARKUP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<[^>]*>|&#?\w+;").expect("regex"));
// Consolidated statement row; `(0)` is the eliminations column, the value is in millions of dollars.
static TOTAL_ASSETS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"Total assets\s*\(0\)\s*([\d,]+)").expect("regex"));
static WEEK_ENDED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"Week ended\s+([A-Za-z]{3})[a-z]*\.?\s+(\d{1,2}),\s+(\d{4})").expect("regex")
});
static RELEASE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"Release Date:\s*([A-Za-z]{3})[a-z]*\.?\s+(\d{1,2}),\s+(\d{4})").expect("regex")
});

struct H41Release {
    release: NaiveDate,
    week_ended: NaiveDate,
    total_assets_usd: f64,
}

fn date(re: &Regex, text: &str) -> Option<NaiveDate> {
    let c = re.captures(text)?;
    let month = MONTHS.iter().position(|m| c[1].eq_ignore_ascii_case(m))? as u32 + 1;
    NaiveDate::from_ymd_opt(c[3].parse().ok()?, month, c[2].parse().ok()?)
}

fn parse_h41(html: &str) -> Option<H41Release> {
    let text = MARKUP.replace_all(html, " ");
    Some(H41Release {
        release: date(&RELEASE, &text)?,
        week_ended: date(&WEEK_ENDED, &text)?,
        total_assets_usd: parse_num(&TOTAL_ASSETS.captures(&text)?[1])? * 1e6,
    })
}

async fn current_h41(app: &App) -> Result<H41Release> {
    let html = app.http.text(&format!("{H41}/current/h41.htm")).await?;
    let h = parse_h41(&html).context("h41: could not parse the current release")?;
    app.store.put(
        "macro.fed_assets_usd",
        day_ts(h.week_ended),
        h.total_assets_usd,
    )?;
    Ok(h)
}

pub async fn poll(app: App) -> Result<()> {
    current_h41(&app).await?;
    tga(&app).await?;
    rrp(&app).await?;
    net_liquidity(&app)
}

/// Twelve prior weekly releases, so the weekly summary has a trend from day one.
pub async fn backfill(app: &App) -> Result<()> {
    let current = current_h41(app).await?;
    for week in 1..=12u64 {
        let week_ended = day_ts(current.week_ended) - week as i64 * 7 * DAY;
        if app
            .store
            .at_or_before("macro.fed_assets_usd", week_ended + DAY, 2 * DAY)?
            .is_some()
        {
            continue;
        }
        // Releases are normally on Thursday; holidays push them a day later.
        let Some(thursday) = current.release.checked_sub_days(Days::new(7 * week)) else {
            continue;
        };
        for release in [Some(thursday), thursday.checked_add_days(Days::new(1))]
            .into_iter()
            .flatten()
        {
            let Ok(html) = app
                .http
                .text(&format!("{H41}/{}/", release.format("%Y%m%d")))
                .await
            else {
                continue;
            };
            if let Some(h) = parse_h41(&html) {
                app.store.put(
                    "macro.fed_assets_usd",
                    day_ts(h.week_ended),
                    h.total_assets_usd,
                )?;
                break;
            }
        }
    }
    tga(app).await?;
    rrp(app).await?;
    net_liquidity(app)
}

async fn tga(app: &App) -> Result<()> {
    let v = app.http.json(TGA).await?;
    let rows: Vec<(i64, f64)> = v["data"]
        .as_array()
        .context("treasury: data missing")?
        .iter()
        .filter(|r| r["account_type"] == TGA_CLOSING)
        .filter_map(|r| {
            let day = NaiveDate::parse_from_str(r["record_date"].as_str()?, "%Y-%m-%d").ok()?;
            // Since the 2022 format change the closing balance is published in this column, in millions.
            Some((day_ts(day), num(&r["open_today_bal"])? * 1e6))
        })
        .collect();
    app.store.put_many("macro.tga_usd", &rows)
}

async fn rrp(app: &App) -> Result<()> {
    let v = app
        .http
        .json(&format!("{RRP}?startDate={}", date_of(now() - 150 * DAY)))
        .await?;
    let mut by_day: BTreeMap<i64, f64> = BTreeMap::new();
    for op in v["repo"]["operations"]
        .as_array()
        .context("nyfed: operations missing")?
    {
        if op["operationType"] != "Reverse Repo" {
            continue;
        }
        let (Some(day), Some(usd)) = (
            op["operationDate"]
                .as_str()
                .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()),
            num(&op["totalAmtAccepted"]),
        ) else {
            continue;
        };
        *by_day.entry(day_ts(day)).or_default() += usd;
    }
    app.store
        .put_many("macro.rrp_usd", &by_day.into_iter().collect::<Vec<_>>())
}

fn net_liquidity(app: &App) -> Result<()> {
    let now = now();
    for (ts, assets) in app
        .store
        .range("macro.fed_assets_usd", now - 200 * DAY, now)?
    {
        let end_of_day = ts + DAY - 1;
        let (Some((_, tga)), Some((_, rrp))) = (
            app.store
                .at_or_before("macro.tga_usd", end_of_day, 10 * DAY)?,
            app.store
                .at_or_before("macro.rrp_usd", end_of_day, 10 * DAY)?,
        ) else {
            continue;
        };
        app.store
            .put("macro.net_liquidity_usd", ts, assets - tga - rrp)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_release_dates_and_total_assets_from_table_markup() {
        let html = r#"<p>Release Date: November 05, 2026</p>
            <th>Week ended<br/>Nov 4, 2026</th>
            <tr><td><p><span>Total assets</span></p></td><td><p><span>(0)</span></p></td>
            <td><p>&#xa0; 6,748,145</p></td><td>+&nbsp;&nbsp;2,510</td></tr>
            <tr><td>Total assets</td><td>6,748,145</td><td>180,053</td></tr>"#;
        let h = parse_h41(html).unwrap();
        assert_eq!(h.release, NaiveDate::from_ymd_opt(2026, 11, 5).unwrap());
        assert_eq!(h.week_ended, NaiveDate::from_ymd_opt(2026, 11, 4).unwrap());
        assert_eq!(h.total_assets_usd, 6_748_145e6);
    }

    #[test]
    fn rejects_pages_without_the_consolidated_statement() {
        assert!(parse_h41("<html>Release Date: November 05, 2026</html>").is_none());
    }
}
