//! Alert rules evaluated once a minute on the indicator snapshot.

use std::time::Duration;

use anyhow::Result;
use tokio::time::interval;

use super::snapshot::{MetricView, Snapshot};
use crate::app::App;
use crate::model::Asset;
use crate::notify::{self, Draft};
use crate::util::{HOUR, date_of, grouped, usd};

pub fn spawn(app: &App) {
    let app = app.clone();
    app.status.expect("rules", 60);
    tokio::spawn(async move {
        let mut tick = interval(Duration::from_secs(60));
        loop {
            tick.tick().await;
            match run(&app) {
                Ok(()) => app.status.ok("rules"),
                Err(e) => {
                    tracing::warn!("rules: {e:#}");
                    app.status.err("rules", format!("{e:#}"));
                }
            }
        }
    });
}

fn run(app: &App) -> Result<()> {
    for asset in Asset::ALL {
        let snap = app.snapshot(asset)?;
        for draft in evaluate(&snap) {
            notify::emit(app, draft)?;
        }
    }
    Ok(())
}

fn z(m: Option<&MetricView>) -> Option<f64> {
    m.and_then(|m| m.z)
}

fn z_text(m: &MetricView) -> String {
    m.z.map_or(String::new(), |z| format!(" (z {z:+.1})"))
}

fn z_or_none(z: Option<f64>) -> String {
    z.map_or("없음".into(), |z| format!("{z:+.1}"))
}

/// One alert per data date for daily metrics.
fn dated(rule: &str, asset: Option<Asset>, m: &MetricView) -> Option<String> {
    Some(format!("{rule}:{}:{}", asset.map_or("all", Asset::key), date_of(m.ts?)))
}

pub fn evaluate(s: &Snapshot) -> Vec<Draft> {
    let a = s.asset;
    let sym = a.sym();
    let mut out = Vec::new();
    let mut push = |tier, severity, rule, asset: Option<Asset>, title: String, body: String, dedupe, cooldown| {
        out.push(Draft { tier, severity, rule, asset, title, body, dedupe, cooldown });
    };

    // ---- 파생: 청산 ----
    if let (Some(long), Some(short)) = (s.get("{a}.liq_long_usd"), s.get("{a}.liq_short_usd")) {
        let (l, sh) = (long.value.unwrap_or(0.0), short.value.unwrap_or(0.0));
        let (absolute, floor) = if a == Asset::Btc { (100e6, 20e6) } else { (60e6, 10e6) };
        let unusual =
            (long.z.is_some_and(|z| z >= 3.0) && l >= floor) || (short.z.is_some_and(|z| z >= 3.0) && sh >= floor);
        if l + sh >= absolute || unusual {
            push(
                "realtime",
                if l + sh >= 3.0 * absolute { "strong" } else { "warn" },
                "liq_spike",
                Some(a),
                format!("{sym} {} 청산 급증", if l >= sh { "롱" } else { "숏" }),
                format!(
                    "1시간 롱 {}{}, 숏 {}{}. Binance, Bybit, OKX 합계",
                    usd(l),
                    z_text(long),
                    usd(sh),
                    z_text(short)
                ),
                None,
                HOUR,
            );
        }
    }

    // ---- 파생: OI ----
    if let Some(oi) = s.get("{a}.oi_usd") {
        if let (Some(oi_1h), Some(px_1h)) = (oi.chg_1h_pct, s.chg_1h_pct)
            && oi_1h >= 3.0
            && px_1h.abs() < 0.7
        {
            push(
                "realtime",
                "warn",
                "oi_buildup",
                Some(a),
                format!("{sym} 가격은 그대로인데 OI 급증"),
                format!("1시간 OI {oi_1h:+.1}%, 가격 {px_1h:+.2}%. 현재 {}", oi.display()),
                None,
                4 * HOUR,
            );
        }
        if let (Some(oi_24h), Some(px_24h)) = (oi.chg_24h_pct, s.chg_24h_pct)
            && oi_24h >= 10.0
            && px_24h.abs() < 2.0
        {
            push(
                "realtime",
                "warn",
                "oi_buildup_24h",
                Some(a),
                format!("{sym} OI가 하루 새 {oi_24h:.0}% 늘어남"),
                format!("가격은 {px_24h:+.2}%. 현재 {}", oi.display()),
                None,
                12 * HOUR,
            );
        }
    }

    // ---- 파생: 펀딩 ----
    if let Some(funding) = s.get("{a}.funding_avg")
        && let Some(v) = funding.value
        && funding.ts.is_some_and(|ts| s.ts - ts <= HOUR)
    {
        let hot = funding.z.is_some_and(|z| z >= 2.5) || v >= 0.05;
        let cold = funding.z.is_some_and(|z| z <= -2.5) || v <= -0.03;
        if hot || cold {
            push(
                "realtime",
                "warn",
                "funding_extreme",
                Some(a),
                format!("{sym} 펀딩비 {}", if hot { "과열" } else { "급락" }),
                format!(
                    "{}{}. {}",
                    funding.display(),
                    z_text(funding),
                    if hot { "롱이 몰려 있음" } else { "숏이 몰려 있음" }
                ),
                None,
                8 * HOUR,
            );
        }
    }

    // ---- 파생: 변동성 ----
    if let Some(dvol) = s.get("{a}.dvol")
        && let Some(jump) = dvol.chg_1h_pct
        && jump >= 8.0
    {
        push(
            "realtime",
            "warn",
            "dvol_jump",
            Some(a),
            format!("{sym} 변동성 급등"),
            format!("DVOL {} (1시간 {jump:+.1}%)", dvol.display()),
            None,
            4 * HOUR,
        );
    }

    // ---- 자금 흐름 ----
    if let Some(premium) = s.get("{a}.coinbase_premium_pct")
        && let (Some(v), Some(z)) = (premium.value, premium.z)
        && z.abs() >= 3.0
        && v.abs() >= 0.1
    {
        push(
            "realtime",
            "info",
            "coinbase_premium",
            Some(a),
            format!("{sym} Coinbase 프리미엄 {}", if v > 0.0 { "급등" } else { "급락" }),
            format!("{}{}", premium.display(), z_text(premium)),
            None,
            4 * HOUR,
        );
    }
    if let Some(kimchi) = s.get("{a}.kimchi_premium_pct")
        && let Some(v) = kimchi.value
        && kimchi.ts.is_some_and(|ts| s.ts - ts <= HOUR)
        && (v >= 5.0 || (kimchi.z.is_some_and(|z| z >= 3.0) && v >= 2.0))
    {
        push(
            "realtime",
            "warn",
            "kimchi_premium",
            Some(a),
            format!("{sym} 김치 프리미엄 급등"),
            format!("{}{}. 국내 과열 신호", kimchi.display(), z_text(kimchi)),
            None,
            12 * HOUR,
        );
    }

    let etf = s.get("etf.{a}.netflow_usd");
    if let Some(etf) = etf.filter(|m| m.value.is_some()) {
        if s.etf_streak <= -3 {
            push(
                "realtime",
                "warn",
                "etf_outflow_streak",
                Some(a),
                format!("{sym} ETF {}일 연속 순유출", -s.etf_streak),
                format!("직전 거래일 {}{}", etf.display(), z_text(etf)),
                dated("etf_outflow_streak", Some(a), etf),
                0,
            );
        }
        if let Some(zv) = etf.z
            && zv.abs() >= 2.0
        {
            push(
                "realtime",
                if zv < 0.0 { "warn" } else { "info" },
                "etf_flow_extreme",
                Some(a),
                format!("{sym} ETF 대규모 {}", if zv < 0.0 { "순유출" } else { "순유입" }),
                format!("직전 거래일 {}, 30일 대비 z {zv:+.1}", etf.display()),
                dated("etf_flow_extreme", Some(a), etf),
                0,
            );
        }
    }

    // ---- 온체인 ----
    let netflow = s.get("{a}.ex_netflow");
    if let Some(netflow) = netflow
        && let (Some(v), Some(zv)) = (netflow.value, netflow.z)
        && zv.abs() >= 2.0
    {
        push(
            "realtime",
            if zv > 0.0 { "warn" } else { "info" },
            "exchange_netflow",
            Some(a),
            format!("{sym} 거래소 {} 급증", if zv > 0.0 { "순입금" } else { "순출금" }),
            format!("{} {sym}, 30일 대비 z {zv:+.1}", grouped(v.abs(), 0)),
            dated("exchange_netflow", Some(a), netflow),
            0,
        );
    }

    // ---- 복합 ----
    let funding_z = z(s.get("{a}.funding_avg"));
    let etf_z = z(etf);
    let netflow_z = z(netflow);
    let legs =
        || format!("ETF z {}, 펀딩 z {}, 순입금 z {}", z_or_none(etf_z), z_or_none(funding_z), z_or_none(netflow_z));
    let etf_out = etf_z.is_some_and(|z| z <= -1.0) || s.etf_streak <= -3;
    if etf_out && funding_z.is_some_and(|z| z >= 1.5) && netflow_z.is_some_and(|z| z >= 1.0) {
        push(
            "composite",
            "strong",
            "composite_bear",
            Some(a),
            format!("{sym} 하락 경고"),
            format!("ETF 유출, 펀딩 과열, 거래소 입금이 겹쳤음. {}", legs()),
            None,
            24 * HOUR,
        );
    }
    if etf_z.is_some_and(|z| z >= 1.0) && funding_z.is_some_and(|z| z <= -1.5) && netflow_z.is_some_and(|z| z <= -1.0) {
        push(
            "composite",
            "strong",
            "composite_bull",
            Some(a),
            format!("{sym} 반등 신호"),
            format!("ETF 유입, 펀딩 음수, 거래소 출금이 겹쳤음. {}", legs()),
            None,
            24 * HOUR,
        );
    }

    // ---- 자산과 무관한 지표는 한 번만 평가 ----
    if a != Asset::Btc {
        return out;
    }

    if let Some(mint) = s.get("stable.net_1d_usd")
        && let (Some(v), Some(zv)) = (mint.value, mint.z)
        && zv.abs() >= 2.5
        && v.abs() >= 500e6
    {
        push(
            "realtime",
            "info",
            "stablecoin_issuance",
            None,
            format!("스테이블코인 대규모 {}", if v > 0.0 { "발행" } else { "소각" }),
            format!("USDT와 USDC 하루 {} (z {zv:+.1})", usd(v)),
            dated("stablecoin_issuance", None, mint),
            0,
        );
    }

    for (key, what) in [("macro.fed.p_cut", "인하"), ("macro.fed.p_hike", "인상")] {
        if let Some(p) = s.get(key)
            && let (Some(v), Some(chg)) = (p.value, p.chg_24h)
            && chg.abs() >= 10.0
        {
            push(
                "realtime",
                "warn",
                "fed_odds_shift",
                None,
                format!("FOMC {what} 확률 급변"),
                format!("{what} {v:.0}%, 하루 새 {chg:+.0}%p. Kalshi 기준"),
                None,
                12 * HOUR,
            );
        }
    }

    // (key, rule, relative change?, threshold, direction that matters, title)
    let shocks: [(&str, &'static str, bool, f64, f64, &str); 6] = [
        ("macro.dxy", "macro_dxy", true, 0.8, 0.0, "달러 급변"),
        ("macro.us10y", "macro_us10y", false, 0.12, 0.0, "미 10년물 금리 급변"),
        ("macro.us2y", "macro_us2y", false, 0.12, 0.0, "미 2년물 금리 급변"),
        ("macro.vix", "macro_vix", true, 20.0, 1.0, "VIX 급등"),
        ("macro.nq", "macro_nq", true, 2.0, -1.0, "나스닥 선물 급락"),
        ("macro.usdjpy", "macro_usdjpy", true, 1.5, -1.0, "엔화 급등"),
    ];
    for (key, rule, relative, threshold, side, title) in shocks {
        let Some(m) = s.get(key) else { continue };
        let Some(chg) = (if relative { m.chg_24h_pct } else { m.chg_24h }) else { continue };
        let hit = if side == 0.0 { chg.abs() >= threshold } else { chg * side >= threshold };
        if hit {
            let moved = if relative { format!("{chg:+.2}%") } else { format!("{:+.0}bp", chg * 100.0) };
            push(
                "realtime",
                "warn",
                rule,
                None,
                title.into(),
                format!("{} {}, 24시간 {moved}", m.label, m.display()),
                None,
                12 * HOUR,
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Store;
    use crate::signal::snapshot::build;
    use crate::util::DAY;

    const NOW: i64 = 100 * DAY + 5 * HOUR;

    /// 30 quiet days followed by `last` yesterday, for a daily metric.
    fn daily(store: &Store, key: &str, quiet: f64, last: &[f64]) {
        let n = last.len() as i64;
        let mut rows: Vec<(i64, f64)> =
            (0..30 - n).map(|i| ((70 + i) * DAY, if i % 2 == 0 { quiet } else { -quiet })).collect();
        rows.extend(last.iter().enumerate().map(|(i, v)| ((100 - n + i as i64) * DAY, *v)));
        store.put_many(key, &rows).unwrap();
    }

    /// A month of hourly values oscillating around `base`, ending at `last` now.
    fn hourly(store: &Store, key: &str, base: f64, wobble: f64, last: f64) {
        let mut rows: Vec<(i64, f64)> =
            (1..=24 * 30).map(|h| (NOW - h * HOUR, base + if h % 2 == 0 { wobble } else { -wobble })).collect();
        rows.push((NOW, last));
        store.put_many(key, &rows).unwrap();
    }

    fn rules(store: &Store, asset: Asset) -> Vec<&'static str> {
        evaluate(&build(store, asset, NOW, false).unwrap()).iter().map(|d| d.rule).collect()
    }

    #[test]
    fn quiet_market_raises_nothing() {
        let s = Store::open(":memory:").unwrap();
        daily(&s, "etf.btc.netflow_usd", 100e6, &[90e6]);
        daily(&s, "btc.ex_netflow", 500.0, &[400.0]);
        hourly(&s, "btc.funding_avg", 0.01, 0.001, 0.0105);
        assert!(rules(&s, Asset::Btc).is_empty());
    }

    #[test]
    fn composite_warning_needs_all_three_legs() {
        let s = Store::open(":memory:").unwrap();
        daily(&s, "etf.btc.netflow_usd", 100e6, &[-150e6, -200e6, -900e6]);
        hourly(&s, "btc.funding_avg", 0.01, 0.001, 0.014);
        // Two legs only: ETF streak and extreme outflow fire, the composite does not.
        let fired = rules(&s, Asset::Btc);
        assert!(fired.contains(&"etf_outflow_streak") && fired.contains(&"etf_flow_extreme"), "{fired:?}");
        assert!(!fired.contains(&"composite_bear"), "{fired:?}");

        daily(&s, "btc.ex_netflow", 500.0, &[6000.0]);
        let fired = rules(&s, Asset::Btc);
        assert!(fired.contains(&"composite_bear") && fired.contains(&"exchange_netflow"), "{fired:?}");
    }

    #[test]
    fn oi_buildup_requires_a_flat_price() {
        let s = Store::open(":memory:").unwrap();
        s.put_many("btc.oi_usd", &[(NOW - HOUR, 7.0e9), (NOW, 7.3e9)]).unwrap();
        s.put_many("btc.price", &[(NOW - HOUR, 70_000.0), (NOW, 70_100.0)]).unwrap();
        assert_eq!(rules(&s, Asset::Btc), ["oi_buildup"]);

        // Same OI jump with a 3% rally is ordinary trend participation.
        s.put("btc.price", NOW, 72_100.0).unwrap();
        assert!(rules(&s, Asset::Btc).is_empty());
    }

    #[test]
    fn global_rules_fire_once_not_per_asset() {
        let s = Store::open(":memory:").unwrap();
        s.put_many("macro.usdjpy", &[(NOW - DAY - 60, 158.0), (NOW - 60, 155.0)]).unwrap();
        assert_eq!(rules(&s, Asset::Btc), ["macro_usdjpy"]);
        assert!(rules(&s, Asset::Eth).is_empty());
    }

    #[test]
    fn liquidation_spike_names_the_side_that_was_hit() {
        let s = Store::open(":memory:").unwrap();
        s.add("eth.liq_long_usd", NOW - 600, 70e6).unwrap();
        s.add("eth.liq_short_usd", NOW - 600, 1e6).unwrap();
        let drafts = evaluate(&build(&s, Asset::Eth, NOW, false).unwrap());
        assert_eq!(drafts.len(), 1);
        assert_eq!((drafts[0].rule, drafts[0].severity), ("liq_spike", "warn"));
        assert_eq!(drafts[0].title, "ETH 롱 청산 급증");
    }
}
