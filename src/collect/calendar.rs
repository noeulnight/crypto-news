//! Scheduled events: this week's US macro releases (Forex Factory feed) and monthly option expiries.

use anyhow::{Context, Result};
use chrono::{DateTime, Datelike, Months, NaiveDate, Weekday};
use serde_json::Value;

use crate::app::App;
use crate::model::Event;
use crate::util::{HOUR, date_of, day_ts, now};

const FEED: &str = "https://nfs.faireconomy.media/ff_calendar_thisweek.json";

pub async fn poll(app: App) -> Result<()> {
    for event in option_expiries(date_of(now())) {
        app.store.upsert_event(&event)?;
    }

    let v = app.http.json(FEED).await?;
    for row in v.as_array().context("calendar: feed is not an array")? {
        if row["country"] != "USD" {
            continue;
        }
        let impact = match row["impact"].as_str() {
            Some("High") => "high",
            Some("Medium") => "medium",
            _ => continue,
        };
        let (Some(title), Some(ts)) = (
            row["title"].as_str(),
            row["date"]
                .as_str()
                .and_then(|d| DateTime::parse_from_rfc3339(d).ok())
                .map(|d| d.timestamp()),
        ) else {
            continue;
        };
        app.store.upsert_event(&Event {
            id: format!("ff:{ts}:{title}"),
            ts,
            title: title.into(),
            kind: "macro".into(),
            impact: impact.into(),
            forecast: text(&row["forecast"]),
            previous: text(&row["previous"]),
        })?;
    }
    Ok(())
}

fn text(v: &Value) -> Option<String> {
    v.as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(Into::into)
}

pub(super) fn last_friday(year: i32, month: u32) -> Option<NaiveDate> {
    let mut day = NaiveDate::from_ymd_opt(year, month, 1)?
        .checked_add_months(Months::new(1))?
        .pred_opt()?;
    while day.weekday() != Weekday::Fri {
        day = day.pred_opt()?;
    }
    Some(day)
}

/// Deribit monthly expiries (last Friday, 08:00 UTC) for this month and the next two.
fn option_expiries(today: NaiveDate) -> Vec<Event> {
    (0..3)
        .filter_map(|i| {
            let month = today.with_day(1)?.checked_add_months(Months::new(i))?;
            let expiry = last_friday(month.year(), month.month())?;
            let quarterly = month.month() % 3 == 0;
            Some(Event {
                id: format!("opt:{}", month.format("%Y-%m")),
                ts: day_ts(expiry) + 8 * HOUR,
                title: if quarterly {
                    "분기 옵션 만기 (Deribit)"
                } else {
                    "월간 옵션 만기 (Deribit)"
                }
                .into(),
                kind: "options_expiry".into(),
                impact: if quarterly { "high" } else { "medium" }.into(),
                forecast: None,
                previous: None,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monthly_expiry_is_the_last_friday() {
        assert_eq!(last_friday(2026, 11), NaiveDate::from_ymd_opt(2026, 11, 27));
        // December rolls into the next year when stepping to the month end.
        assert_eq!(last_friday(2026, 12), NaiveDate::from_ymd_opt(2026, 12, 25));
        // A month that ends on a Friday.
        assert_eq!(last_friday(2026, 7), NaiveDate::from_ymd_opt(2026, 7, 31));
    }

    #[test]
    fn expiries_cover_three_months_and_flag_quarter_ends() {
        let events = option_expiries(NaiveDate::from_ymd_opt(2026, 11, 30).unwrap());
        let ids: Vec<&str> = events.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["opt:2026-11", "opt:2026-12", "opt:2027-01"]);
        assert_eq!(events[1].impact, "high");
        assert_eq!(events[0].impact, "medium");
    }
}
