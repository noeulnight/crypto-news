use chrono::{DateTime, FixedOffset, NaiveDate, Utc};
use serde_json::Value;

pub const HOUR: i64 = 3600;
pub const DAY: i64 = 86_400;

pub fn now() -> i64 {
    Utc::now().timestamp()
}

pub fn floor_to(ts: i64, step: i64) -> i64 {
    ts - ts.rem_euclid(step)
}

/// Parses a JSON number or a numeric string such as `"73,830.00"`, `"4.748%"`, `"+0.06%"`.
pub fn num(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => parse_num(s),
        _ => None,
    }
}

pub fn parse_num(s: &str) -> Option<f64> {
    let cleaned: String = s.chars().filter(|c| !matches!(c, ',' | '%' | '$' | ' ' | '+')).collect();
    cleaned.parse::<f64>().ok().filter(|x| x.is_finite())
}

pub fn kst(ts: i64) -> DateTime<FixedOffset> {
    let offset = FixedOffset::east_opt(9 * 3600).expect("valid offset");
    DateTime::from_timestamp(ts, 0).unwrap_or_default().with_timezone(&offset)
}

pub fn day_ts(date: NaiveDate) -> i64 {
    date.and_hms_opt(0, 0, 0).expect("midnight").and_utc().timestamp()
}

pub fn date_of(ts: i64) -> NaiveDate {
    DateTime::from_timestamp(ts, 0).unwrap_or_default().date_naive()
}

pub fn pct(new: f64, old: f64) -> Option<f64> {
    (old != 0.0).then(|| (new / old - 1.0) * 100.0)
}

/// Compact USD amount: `$1.23B`, `-$350.0M`, `$12.3K`.
pub fn usd(v: f64) -> String {
    let sign = if v < 0.0 { "-" } else { "" };
    let a = v.abs();
    if a >= 1e12 {
        format!("{sign}${:.2}T", a / 1e12)
    } else if a >= 1e9 {
        format!("{sign}${:.2}B", a / 1e9)
    } else if a >= 1e6 {
        format!("{sign}${:.1}M", a / 1e6)
    } else if a >= 1e3 {
        format!("{sign}${:.1}K", a / 1e3)
    } else {
        format!("{sign}${a:.0}")
    }
}

/// Fixed decimals with thousands separators: `28,268`, `-1,234.5`.
pub fn grouped(v: f64, decimals: usize) -> String {
    let digits = format!("{:.decimals$}", v.abs());
    let (int, frac) = digits.split_once('.').unwrap_or((&digits, ""));
    let mut out = String::with_capacity(digits.len() + int.len() / 3 + 1);
    // A value that rounds to zero carries no sign.
    if v < 0.0 && digits.bytes().any(|b| (b'1'..=b'9').contains(&b)) {
        out.push('-');
    }
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if !frac.is_empty() {
        out.push('.');
        out.push_str(frac);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grouped_separates_thousands_and_keeps_decimals() {
        assert_eq!(grouped(28268.0, 0), "28,268");
        assert_eq!(grouped(-1234.56, 1), "-1,234.6");
        assert_eq!(grouped(999.0, 0), "999");
        assert_eq!(grouped(1_000_000.0, 2), "1,000,000.00");
        assert_eq!(grouped(0.0097, 4), "0.0097");
        // Rounds to zero: no stray minus sign.
        assert_eq!(grouped(-0.4, 0), "0");
    }
}
