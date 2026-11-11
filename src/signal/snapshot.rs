//! Point-in-time view of every catalog metric for one asset, plus the heuristic scores.

use anyhow::Result;
use serde::Serialize;

use super::catalog::{Basis, Cadence, Category, DEFS, Def, Kind};
use super::stats::{lagged_changes, zscore};
use crate::db::Store;
use crate::model::Asset;
use crate::util::{DAY, HOUR, floor_to, grouped, pct, usd};

pub const Z_WINDOW: i64 = 30 * DAY;
const SPARK_HOURS: usize = 168;
const SPARK_DAYS: usize = 60;

#[derive(Clone, Serialize)]
pub struct MetricView {
    pub key: String,
    pub label: &'static str,
    pub category: Category,
    pub unit: &'static str,
    pub decimals: u8,
    pub cadence: Cadence,
    pub value: Option<f64>,
    pub ts: Option<i64>,
    pub chg_1h: Option<f64>,
    /// Day-over-day for daily metrics, week-over-week for weekly ones.
    pub chg_24h: Option<f64>,
    pub z: Option<f64>,
    pub signal: &'static str,
    pub note: &'static str,
    pub spark: Vec<(i64, f64)>,
    /// `z`, or the scaled change while history is too short for a real z-score.
    #[serde(skip)]
    pub z_eff: Option<f64>,
    #[serde(skip)]
    pub chg_1h_pct: Option<f64>,
    #[serde(skip)]
    pub chg_24h_pct: Option<f64>,
    #[serde(skip)]
    pub dir: f64,
    #[serde(skip)]
    pub weight: f64,
    /// Older than its cadence allows (or never seen); not usable as evidence.
    #[serde(skip)]
    pub stale: bool,
}

impl MetricView {
    /// Human-readable current value, e.g. `0.0097%`, `-$220.6M`, `28,268`.
    pub fn display(&self) -> String {
        let d = self.decimals as usize;
        match (self.value, self.unit) {
            (None, _) => "없음".into(),
            (Some(v), "$") => usd(v),
            (Some(v), "%" | "bp") => format!("{}{}", grouped(v, d), self.unit),
            (Some(v), _) => grouped(v, d),
        }
    }

    /// Contribution to the score: positive is bullish.
    pub fn pull(&self) -> Option<f64> {
        (self.dir != 0.0).then_some(self.z_eff? * self.dir)
    }
}

#[derive(Clone, Default, Serialize)]
pub struct Scores {
    pub total: Option<f64>,
    pub flow: Option<f64>,
    pub derivs: Option<f64>,
    #[serde(rename = "macro")]
    pub macro_: Option<f64>,
    pub onchain: Option<f64>,
}

pub struct Snapshot {
    pub asset: Asset,
    pub ts: i64,
    pub price: Option<f64>,
    pub chg_1h_pct: Option<f64>,
    pub chg_24h_pct: Option<f64>,
    pub scores: Scores,
    pub metrics: Vec<MetricView>,
    /// Consecutive settled days of ETF inflow (positive) or outflow (negative).
    pub etf_streak: i32,
}

impl Snapshot {
    /// Looks a metric up by its catalog key template, e.g. `"{a}.funding_avg"`.
    pub fn get(&self, template: &str) -> Option<&MetricView> {
        let key = template.replace("{a}", self.asset.key());
        self.metrics.iter().find(|m| m.key == key)
    }
}

pub fn build(store: &Store, asset: Asset, now: i64, whale: bool) -> Result<Snapshot> {
    let price_key = asset.m("price");
    let price = store
        .latest(&price_key)?
        .filter(|&(ts, _)| now - ts <= 10 * 60)
        .map(|(_, p)| p);
    let change = |ago: i64, tol: i64| -> Result<Option<f64>> {
        let Some(price) = price else { return Ok(None) };
        Ok(store
            .at_or_before(&price_key, now - ago, tol)?
            .and_then(|(_, old)| pct(price, old)))
    };
    let chg_1h_pct = change(HOUR, 15 * 60)?;
    let chg_24h_pct = change(DAY, 2 * HOUR)?;

    let mut metrics = Vec::with_capacity(DEFS.len());
    for def in DEFS.iter().filter(|d| whale || !d.whale) {
        metrics.push(view(store, def, asset, now)?);
    }
    let etf = store.range(
        &format!("etf.{}.netflow_usd", asset.key()),
        now - 60 * DAY,
        floor_to(now, DAY) - 1,
    )?;
    Ok(Snapshot {
        asset,
        ts: now,
        price,
        chg_1h_pct,
        chg_24h_pct,
        scores: score(&metrics),
        metrics,
        etf_streak: streak(&etf),
    })
}

fn view(store: &Store, def: &Def, asset: Asset, now: i64) -> Result<MetricView> {
    let mut v = MetricView {
        key: def.key_for(asset),
        label: def.label,
        category: def.category,
        unit: def.unit,
        decimals: def.decimals,
        cadence: def.cadence,
        value: None,
        ts: None,
        chg_1h: None,
        chg_24h: None,
        z: None,
        signal: "neutral",
        note: def.note,
        spark: Vec::new(),
        z_eff: None,
        chg_1h_pct: None,
        chg_24h_pct: None,
        dir: def.dir,
        weight: def.weight,
        stale: true,
    };
    match (def.cadence, def.kind) {
        (Cadence::Intraday, Kind::Sum1h) => rolling_sum(store, &mut v, now)?,
        (Cadence::Intraday, Kind::Level) => intraday(store, def, &mut v, now)?,
        (Cadence::Daily, _) => daily(store, def, &mut v, now)?,
        (Cadence::Weekly, _) => weekly(store, &mut v, now)?,
    }
    v.signal = match v.pull() {
        Some(p) if p >= 1.0 => "bullish",
        Some(p) if p <= -1.0 => "bearish",
        _ => "neutral",
    };
    Ok(v)
}

fn tail(series: &[(i64, f64)], n: usize) -> Vec<(i64, f64)> {
    series[series.len().saturating_sub(n)..].to_vec()
}

fn set_basis_z(
    v: &mut MetricView,
    basis: Basis,
    value: f64,
    levels: &[f64],
    changes: &[f64],
    min_n: (usize, usize),
) {
    match basis {
        Basis::Level => {
            v.z = zscore(value, levels, min_n.0);
            v.z_eff = v.z;
        }
        Basis::ChgPct(scale) | Basis::ChgAbs(scale) => {
            let current = if matches!(basis, Basis::ChgPct(_)) {
                v.chg_24h_pct
            } else {
                v.chg_24h
            };
            if let Some(current) = current {
                v.z = zscore(current, changes, min_n.1);
                v.z_eff = v.z.or(Some(current / scale));
            }
        }
    }
}

fn intraday(store: &Store, def: &Def, v: &mut MetricView, now: i64) -> Result<()> {
    let Some((ts, value)) = store.latest(&v.key)? else {
        return Ok(());
    };
    v.value = Some(value);
    v.ts = Some(ts);
    let hourly = store.bucket_last(&v.key, now - Z_WINDOW - DAY, now, HOUR)?;
    v.spark = tail(&hourly, SPARK_HOURS)
        .into_iter()
        .filter(|p| p.0 >= now - 7 * DAY)
        .collect();

    // Macro markets close overnight and over weekends: stay usable across the gap and measure
    // the day change against the last print before it.
    let (fresh_for, day_tol) = if def.category == Category::Macro {
        (4 * DAY, 4 * DAY)
    } else {
        (6 * HOUR, 3 * HOUR)
    };
    if now - ts > fresh_for {
        return Ok(());
    }
    v.stale = false;
    if let Some((_, old)) = store.at_or_before(&v.key, now - HOUR, 30 * 60)? {
        v.chg_1h = Some(value - old);
        v.chg_1h_pct = pct(value, old);
    }
    if let Some((_, old)) = store.at_or_before(&v.key, now - DAY, day_tol)? {
        v.chg_24h = Some(value - old);
        v.chg_24h_pct = pct(value, old);
    }
    let levels: Vec<f64> = hourly.iter().filter(|p| p.0 < ts).map(|p| p.1).collect();
    let relative = matches!(def.basis, Basis::ChgPct(_));
    let changes: Vec<f64> = match def.basis {
        Basis::Level => Vec::new(),
        _ => lagged_changes(&hourly, DAY, day_tol, relative)
            .into_iter()
            .map(|p| p.1)
            .collect(),
    };
    // A real z-score needs at least a day of levels, or about a week of day-changes.
    set_basis_z(v, def.basis, value, &levels, &changes, (24, 120));
    Ok(())
}

/// Hourly sums from the first observed hour to now, with empty hours as zeros.
pub fn hourly_sums(store: &Store, key: &str, from: i64, now: i64) -> Result<Vec<(i64, f64)>> {
    let buckets = store.bucket_sum(key, from, now, HOUR)?;
    let Some(&(first, _)) = buckets.first() else {
        return Ok(Vec::new());
    };
    let mut it = buckets.into_iter().peekable();
    let mut out = Vec::new();
    let mut hour = first;
    while hour <= floor_to(now, HOUR) {
        out.push((hour, it.next_if(|b| b.0 == hour).map_or(0.0, |b| b.1)));
        hour += HOUR;
    }
    Ok(out)
}

fn rolling_sum(store: &Store, v: &mut MetricView, now: i64) -> Result<()> {
    let hourly = hourly_sums(store, &v.key, now - Z_WINDOW, now)?;
    // Nothing recorded yet: unknown rather than zero.
    if hourly.is_empty() {
        return Ok(());
    }
    let value = store.sum(&v.key, now - HOUR + 1, now)?;
    let previous = store.sum(&v.key, now - 2 * HOUR + 1, now - HOUR)?;
    v.value = Some(value);
    v.ts = Some(now);
    v.stale = false;
    v.chg_1h = Some(value - previous);
    let complete: Vec<f64> = hourly
        .iter()
        .filter(|p| p.0 < floor_to(now, HOUR))
        .map(|p| p.1)
        .collect();
    v.z = zscore(value, &complete, 72);
    v.z_eff = v.z;
    v.spark = tail(&hourly, SPARK_HOURS);
    Ok(())
}

fn daily(store: &Store, def: &Def, v: &mut MetricView, now: i64) -> Result<()> {
    // The current UTC day is still accumulating (ETF issuers report late, supply moves intraday).
    let settled = store.range(&v.key, now - 95 * DAY, floor_to(now, DAY) - 1)?;
    let Some(&(ts, value)) = settled.last() else {
        return Ok(());
    };
    v.value = Some(value);
    v.ts = Some(ts);
    v.spark = tail(&settled, SPARK_DAYS);
    // Long weekends leave up to five days between prints.
    if now - ts > 6 * DAY {
        return Ok(());
    }
    v.stale = false;
    if let Some(&(_, old)) = settled.iter().rev().nth(1) {
        v.chg_24h = Some(value - old);
        v.chg_24h_pct = pct(value, old);
    }
    let window: Vec<(i64, f64)> = settled
        .iter()
        .copied()
        .filter(|p| p.0 >= ts - Z_WINDOW)
        .collect();
    let prior = &window[..window.len() - 1];
    let levels: Vec<f64> = prior.iter().map(|p| p.1).collect();
    let relative = matches!(def.basis, Basis::ChgPct(_));
    let changes: Vec<f64> = prior
        .windows(2)
        .filter_map(|w| {
            if relative {
                pct(w[1].1, w[0].1)
            } else {
                Some(w[1].1 - w[0].1)
            }
        })
        .collect();
    set_basis_z(v, def.basis, value, &levels, &changes, (10, 10));
    Ok(())
}

fn weekly(store: &Store, v: &mut MetricView, now: i64) -> Result<()> {
    let series = store.range(&v.key, now - 400 * DAY, now)?;
    let Some(&(ts, value)) = series.last() else {
        return Ok(());
    };
    v.value = Some(value);
    v.ts = Some(ts);
    if let Some(&(_, old)) = series.iter().rev().nth(1) {
        v.chg_24h = Some(value - old);
        v.chg_24h_pct = pct(value, old);
    }
    v.spark = tail(&series, SPARK_DAYS);
    v.stale = now - ts > 14 * DAY;
    Ok(())
}

/// Each scored metric contributes its clipped z (±3 maps to ±100), weighted within its category.
fn score(metrics: &[MetricView]) -> Scores {
    let category = |c: Category| -> Option<f64> {
        let (mut sum, mut weight) = (0.0, 0.0);
        for m in metrics.iter().filter(|m| m.category == c) {
            if let Some(pull) = m.pull() {
                sum += m.weight * pull.clamp(-3.0, 3.0) / 3.0;
                weight += m.weight;
            }
        }
        (weight > 0.0).then(|| 100.0 * sum / weight)
    };
    let mut s = Scores {
        flow: category(Category::Flow),
        derivs: category(Category::Derivs),
        macro_: category(Category::Macro),
        onchain: category(Category::Onchain),
        total: None,
    };
    let parts = [
        (Category::Flow, s.flow),
        (Category::Derivs, s.derivs),
        (Category::Macro, s.macro_),
        (Category::Onchain, s.onchain),
    ];
    let weight: f64 = parts
        .iter()
        .filter(|p| p.1.is_some())
        .map(|p| p.0.weight())
        .sum();
    if weight > 0.0 {
        s.total = Some(
            parts
                .iter()
                .filter_map(|(c, v)| Some(c.weight() * (*v)?))
                .sum::<f64>()
                / weight,
        );
    }
    s
}

fn streak(daily: &[(i64, f64)]) -> i32 {
    let Some(sign) = daily.last().map(|p| p.1.signum()).filter(|s| *s != 0.0) else {
        return 0;
    };
    let days = daily
        .iter()
        .rev()
        .take_while(|p| p.1 != 0.0 && p.1.signum() == sign)
        .count() as i32;
    if sign > 0.0 { days } else { -days }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        Store::open(":memory:").unwrap()
    }

    #[test]
    fn streak_counts_consecutive_days_on_the_latest_side() {
        assert_eq!(streak(&[(1, 5.0), (2, -1.0), (3, -2.0), (4, -3.0)]), -3);
        assert_eq!(streak(&[(1, -5.0), (2, 1.0), (3, 2.0)]), 2);
        assert_eq!(streak(&[(1, 1.0), (2, 0.0)]), 0);
        assert_eq!(streak(&[]), 0);
    }

    #[test]
    fn daily_metric_ignores_the_unsettled_current_day() {
        let s = store();
        let now = 100 * DAY + 5 * HOUR;
        // 20 settled days alternating around zero, then a large outflow yesterday and a partial print today.
        let mut rows: Vec<(i64, f64)> = (0..20)
            .map(|i| ((78 + i) * DAY, if i % 2 == 0 { 100.0 } else { -100.0 }))
            .collect();
        rows.push((98 * DAY, -100.0));
        rows.push((99 * DAY, -900.0));
        rows.push((100 * DAY, 5000.0));
        s.put_many("etf.btc.netflow_usd", &rows).unwrap();

        let snap = build(&s, Asset::Btc, now, false).unwrap();
        let etf = snap.get("etf.{a}.netflow_usd").unwrap();
        assert_eq!(etf.value, Some(-900.0));
        assert_eq!(etf.ts, Some(99 * DAY));
        assert_eq!(etf.chg_24h, Some(-800.0));
        assert!(etf.z.unwrap() < -3.0, "z = {:?}", etf.z);
        assert_eq!(etf.signal, "bearish");
        assert_eq!(snap.etf_streak, -3);
        // The only scored flow metric is at the clip, so the category sits at its floor.
        assert_eq!(snap.scores.flow, Some(-100.0));
        assert_eq!(snap.scores.total, Some(-100.0));
    }

    #[test]
    fn contrarian_metrics_score_against_their_z() {
        let s = store();
        let now = 40 * DAY;
        // A month of flat-ish funding, then a spike.
        let mut rows: Vec<(i64, f64)> = (1..=24 * 30)
            .map(|h| (now - h * HOUR, if h % 2 == 0 { 0.009 } else { 0.011 }))
            .collect();
        rows.push((now, 0.05));
        s.put_many("btc.funding_avg", &rows).unwrap();

        let snap = build(&s, Asset::Btc, now, false).unwrap();
        let funding = snap.get("{a}.funding_avg").unwrap();
        assert!(funding.z.unwrap() > 3.0);
        assert_eq!(funding.signal, "bearish");
        assert_eq!(snap.scores.derivs, Some(-100.0));
    }

    #[test]
    fn rolling_sum_fills_quiet_hours_with_zeros_and_stays_unknown_before_first_event() {
        let s = store();
        let now = 10 * DAY + 30 * 60;
        let empty = build(&s, Asset::Btc, now, false).unwrap();
        assert_eq!(empty.get("{a}.liq_long_usd").unwrap().value, None);

        // One liquidation 5 hours ago, one 10 minutes ago.
        s.add("btc.liq_long_usd", now - 5 * HOUR, 1e6).unwrap();
        s.add("btc.liq_long_usd", now - 10 * 60, 4e6).unwrap();
        let snap = build(&s, Asset::Btc, now, false).unwrap();
        let liq = snap.get("{a}.liq_long_usd").unwrap();
        assert_eq!(liq.value, Some(4e6));
        assert_eq!(liq.chg_1h, Some(4e6));
        let sums: Vec<f64> = liq.spark.iter().map(|p| p.1).collect();
        assert_eq!(sums, [1e6, 0.0, 0.0, 0.0, 0.0, 4e6]);
    }

    #[test]
    fn trending_metric_falls_back_to_scaled_change_until_history_exists() {
        let s = store();
        let now = 50 * DAY;
        s.put_many("macro.dxy", &[(now - DAY - 60, 100.0), (now - 60, 100.8)])
            .unwrap();
        let snap = build(&s, Asset::Btc, now, false).unwrap();
        let dxy = snap.get("macro.dxy").unwrap();
        assert_eq!(dxy.z, None);
        // +0.8% against a 0.4% typical move: two "sigmas" of dollar strength, bearish.
        assert!((dxy.z_eff.unwrap() - 2.0).abs() < 1e-6);
        assert_eq!(dxy.signal, "bearish");
    }
}
