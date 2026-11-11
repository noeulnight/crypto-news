//! Scheduled channel: morning briefing (08:30 KST), weekly summary (Monday) and event reminders.

use std::time::Duration;

use anyhow::Result;
use chrono::{Datelike, Days, NaiveDate};
use tokio::time::interval;

use crate::app::App;
use crate::model::{Asset, bias_ko, regime_ko};
use crate::notify::{self, Draft};
use crate::signal::snapshot::{MetricView, Snapshot};
use crate::util::{DAY, HOUR, floor_to, grouped, kst, now, usd};

pub fn spawn(app: &App) {
    let app = app.clone();
    app.status.expect("brief", 30);
    tokio::spawn(async move {
        let mut tick = interval(Duration::from_secs(30));
        loop {
            tick.tick().await;
            match run(&app) {
                Ok(()) => app.status.ok("brief"),
                Err(e) => {
                    tracing::warn!("brief: {e:#}");
                    app.status.err("brief", format!("{e:#}"));
                }
            }
        }
    });
}

fn kst_ts(date: NaiveDate, hour: u32, minute: u32) -> i64 {
    date.and_hms_opt(hour, minute, 0).expect("valid time").and_utc().timestamp() - 9 * HOUR
}

fn scheduled(rule: &'static str, title: String, body: String, dedupe: Option<String>) -> Draft {
    Draft { tier: "scheduled", severity: "info", rule, asset: None, title, body, dedupe, cooldown: 0 }
}

fn run(app: &App) -> Result<()> {
    let now = now();
    let today = kst(now).date_naive();

    // If the app was down at 08:30 the briefing is produced on the next start the same day.
    let due = kst_ts(today, 8, 30);
    if now >= due && app.store.latest_brief("morning")?.is_none_or(|b| b.ts < due) {
        let body = morning(app, now)?;
        app.store.put_brief("morning", now, &body)?;
        notify::emit(app, scheduled("brief_morning", "아침 브리핑".into(), body, None))?;
    }

    let monday = today.checked_sub_days(Days::new(today.weekday().num_days_from_monday().into())).unwrap_or(today);
    let mut due = kst_ts(monday, 8, 35);
    if due > now {
        due -= 7 * DAY;
    }
    if app.store.latest_brief("weekly")?.is_none_or(|b| b.ts < due) {
        let body = weekly(app, now)?;
        app.store.put_brief("weekly", now, &body)?;
        notify::emit(app, scheduled("brief_weekly", "주간 요약".into(), body, None))?;
    }

    for e in app.store.events(now + 1, now + 30 * 60)?.into_iter().filter(|e| e.impact == "high") {
        let mut body = format!("{} 발표", kst(e.ts).format("%H:%M"));
        if let Some(f) = &e.forecast {
            body.push_str(&format!(", 예상 {f}"));
        }
        if let Some(p) = &e.previous {
            body.push_str(&format!(", 이전 {p}"));
        }
        let title = format!("{}분 뒤 {}", (e.ts - now + 59) / 60, e.title);
        notify::emit(app, scheduled("event_reminder", title, body, Some(format!("remind:{}", e.id))))?;
    }
    Ok(())
}

fn score(v: Option<f64>) -> String {
    v.map_or("없음".to_owned(), |v| format!("{v:+.0}"))
}

/// `value (z +1.2)`, or `None` when the metric has no data.
fn show(m: Option<&MetricView>) -> Option<String> {
    let m = m.filter(|m| m.value.is_some())?;
    Some(format!("{}{}", m.display(), m.z.map_or(String::new(), |z| format!(" (z {z:+.1})"))))
}

/// `name value (+0.3%)`; yields are quoted in percent, so their move is shown in basis points.
fn macro_item(s: &Snapshot, key: &str, name: &str) -> Option<String> {
    let m = s.get(key).filter(|m| m.value.is_some())?;
    let change = match (m.unit, m.chg_24h, m.chg_24h_pct) {
        ("%", Some(abs), _) => format!(" ({:+.0}bp)", abs * 100.0),
        (_, _, Some(rel)) => format!(" ({rel:+.2}%)"),
        _ => String::new(),
    };
    Some(format!("{name} {}{change}", m.display()))
}

fn morning(app: &App, now: i64) -> Result<String> {
    let snaps = [app.snapshot(Asset::Btc)?, app.snapshot(Asset::Eth)?];
    let global = &snaps[0];
    let mut lines = vec![format!("{} 기준", kst(now).format("%-m/%-d %H:%M"))];
    let mut section = |title: &str, body: Vec<String>| {
        if !body.is_empty() {
            lines.push(String::new());
            lines.push(title.into());
            lines.extend(body);
        }
    };

    let mut body = Vec::new();
    for s in &snaps {
        let Some(price) = s.price else { continue };
        body.push(format!(
            "{} ${} (24시간 {}), 종합 점수 {}",
            s.asset.sym(),
            grouped(price, 0),
            s.chg_24h_pct.map_or("없음".to_owned(), |c| format!("{c:+.2}%")),
            score(s.scores.total),
        ));
    }
    section("시세", body);

    let mut body = Vec::new();
    for s in &snaps {
        let sym = s.asset.sym();
        if let Some(etf) = s.get("etf.{a}.netflow_usd")
            && let (Some(text), Some(ts)) = (show(Some(etf)), etf.ts)
        {
            let streak = match s.etf_streak {
                n if n <= -2 => format!(", {}일 연속 유출", -n),
                n if n >= 2 => format!(", {n}일 연속 유입"),
                _ => String::new(),
            };
            // Daily points sit at 00:00 UTC of the US trading day.
            body.push(format!("{sym} ETF {text}, {} 거래분{streak}", kst(ts + 12 * HOUR).format("%-m/%-d")));
        }
        let premiums: Vec<String> =
            [("Coinbase 프리미엄", "{a}.coinbase_premium_pct"), ("김치 프리미엄", "{a}.kimchi_premium_pct")]
                .into_iter()
                .filter_map(|(name, key)| Some(format!("{name} {}", show(s.get(key))?)))
                .collect();
        if !premiums.is_empty() {
            body.push(format!("{sym} {}", premiums.join(", ")));
        }
    }
    body.extend(show(global.get("stable.net_7d_usd")).map(|t| format!("스테이블코인 7일 순발행 {t}")));
    section("자금", body);

    let mut body = Vec::new();
    for s in &snaps {
        let mut parts = Vec::new();
        parts.extend(show(s.get("{a}.funding_avg")).map(|t| format!("펀딩 {t}")));
        if let Some(oi) = s.get("{a}.oi_usd").filter(|m| m.value.is_some()) {
            let change = oi.chg_24h_pct.map_or(String::new(), |c| format!(" (24시간 {c:+.1}%)"));
            parts.push(format!("OI {}{change}", oi.display()));
        }
        if let (Some(long), Some(short)) =
            (s.get("{a}.liq_long_usd").and_then(|m| m.value), s.get("{a}.liq_short_usd").and_then(|m| m.value))
        {
            parts.push(format!("1시간 청산 롱 {} 숏 {}", usd(long), usd(short)));
        }
        parts.extend(show(s.get("{a}.dvol")).map(|t| format!("DVOL {t}")));
        if !parts.is_empty() {
            body.push(format!("{} {}", s.asset.sym(), parts.join(", ")));
        }
    }
    section("파생", body);

    let mut body: Vec<String> = [
        ("macro.dxy", "달러 인덱스"),
        ("macro.us10y", "미 10년"),
        ("macro.us2y", "미 2년"),
        ("macro.nq", "나스닥 선물"),
        ("macro.vix", "VIX"),
        ("macro.usdjpy", "USD/JPY"),
    ]
    .into_iter()
    .filter_map(|(key, name)| macro_item(global, key, name))
    .collect();
    if let (Some(cut), Some(hike), Some(bp)) = (
        global.get("macro.fed.p_cut").and_then(|m| m.value),
        global.get("macro.fed.p_hike").and_then(|m| m.value),
        global.get("macro.fed.expected_bp").and_then(|m| m.value),
    ) {
        body.push(format!("다음 FOMC 인하 {cut:.0}%, 인상 {hike:.0}% (기대 {bp:+.0}bp)"));
    }
    section("거시", body);

    let mut body = Vec::new();
    for s in &snaps {
        if let Some(netflow) = s.get("{a}.ex_netflow")
            && let Some(v) = netflow.value
        {
            let z = netflow.z.map_or(String::new(), |z| format!(" (z {z:+.1})"));
            let side = if v >= 0.0 { "순입금" } else { "순출금" };
            body.push(format!("{} 거래소 {side} {} {}{z}", s.asset.sym(), grouped(v.abs(), 0), s.asset.sym()));
        }
    }
    section("온체인", body);

    let mut body = Vec::new();
    for e in app.store.events(now, now + DAY)? {
        let mut line = format!("{} {}", kst(e.ts).format("%-m/%-d %H:%M"), e.title);
        if let (Some(f), Some(p)) = (&e.forecast, &e.previous) {
            line.push_str(&format!(" (예상 {f}, 이전 {p})"));
        }
        body.push(line);
    }
    if body.is_empty() {
        body.push("24시간 안에 잡힌 일정 없음".into());
    }
    section("일정", body);

    let mut body = Vec::new();
    for asset in Asset::ALL {
        // Prefer Claude's judgment when there is one.
        let latest = match app.store.judgments(asset.key(), Some("claude"), 1)?.pop() {
            Some(j) => Some(j),
            None => app.store.judgments(asset.key(), Some("rules"), 1)?.pop(),
        };
        if let Some(j) = latest {
            body.push(format!(
                "{} {}, 4시간 {}, 24시간 {}",
                asset.sym(),
                regime_ko(&j.regime),
                bias_ko(&j.bias_4h),
                bias_ko(&j.bias_24h),
            ));
        }
    }
    section("판정", body);

    Ok(lines.join("\n"))
}

fn weekly(app: &App, now: i64) -> Result<String> {
    let s = app.snapshot(Asset::Btc)?;
    let mut lines = vec![format!("{} 기준", kst(now).format("%-m/%-d %H:%M"))];
    let mut section = |title: &str, body: Vec<String>| {
        if !body.is_empty() {
            lines.push(String::new());
            lines.push(title.into());
            lines.extend(body);
        }
    };

    let mut body = Vec::new();
    match s.get("macro.net_liquidity_usd").filter(|m| m.value.is_some()) {
        Some(net) => {
            let change = net.chg_24h.map_or(String::new(), |c| format!(", 전주 대비 {}", usd(c)));
            body.push(format!("{}{change}", net.display()));
            let parts: Vec<String> =
                [("연준 자산", "macro.fed_assets_usd"), ("TGA", "macro.tga_usd"), ("RRP", "macro.rrp_usd")]
                    .into_iter()
                    .filter_map(|(name, key)| Some(format!("{name} {}", usd(app.store.latest(key).ok()??.1))))
                    .collect();
            body.push(parts.join(", "));
        }
        None => body.push("아직 수집 전".into()),
    }
    section("연준 순유동성", body);

    let mut body = Vec::new();
    for asset in Asset::ALL {
        let settled =
            app.store.range(&format!("etf.{}.netflow_usd", asset.key()), now - 14 * DAY, floor_to(now, DAY) - 1)?;
        let last5 = &settled[settled.len().saturating_sub(5)..];
        if !last5.is_empty() {
            body.push(format!("{} {}", asset.sym(), usd(last5.iter().map(|p| p.1).sum())));
        }
    }
    section("현물 ETF 최근 5거래일", body);

    let body = s
        .get("stable.net_7d_usd")
        .filter(|m| m.value.is_some())
        .map(|m| format!("USDT와 USDC 7일 순발행 {}", m.display()))
        .into_iter()
        .collect();
    section("스테이블코인", body);

    let mut body = Vec::new();
    let recent = app.store.judgments(Asset::Btc.key(), None, 4000)?;
    for (source, label) in [("claude", "Claude"), ("rules", "룰")] {
        let hits: Vec<bool> = recent
            .iter()
            .filter(|j| j.source == source && j.ts >= now - 7 * DAY && j.bias_4h != "neutral")
            .filter_map(|j| j.hit_4h)
            .collect();
        if hits.is_empty() {
            body.push(format!("{label} 채점된 판정 없음"));
        } else {
            body.push(format!("{label} {}번 중 {}번 적중", hits.len(), hits.iter().filter(|h| **h).count()));
        }
    }
    section("지난 7일 BTC 4시간 판정", body);

    Ok(lines.join("\n"))
}
