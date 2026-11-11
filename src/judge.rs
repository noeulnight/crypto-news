//! Hourly momentum judgment (Claude, plus a rule-based control) and the ledger that scores both
//! against what the price did 4 and 24 hours later.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::time::interval;

use crate::app::{App, BusEvent};
use crate::claude::Claude;
use crate::db::{Horizon, Store};
use crate::model::{
    Asset, BIASES, CONFIDENCES, DIRECTIONS, Driver, Event, Judgment, REGIMES, Verdict,
};
use crate::signal::snapshot::{self, MetricView, Snapshot};
use crate::util::{HOUR, floor_to, kst, now, pct};

const JUDGE_SYSTEM: &str = "너는 암호화폐 단기 시장 국면을 판정하는 분석가다. 입력 JSON에는 코드가 계산한 \
지표 스냅샷(값, 1시간과 24시간 변화, 30일 z-score, 방향 신호), 카테고리 점수(-100에서 +100, 양수가 강세), \
최근 뉴스, 예정 일정, 직전 판정과 그 채점 결과가 들어 있다.\n\
\n\
판정 규칙\n\
- 입력에 있는 데이터만 근거로 쓴다. value가 null이거나 stale이 true인 지표는 쓰지 않는다. \
cadence가 daily나 weekly인 지표는 원래 하루나 한 주에 한 번 나오므로 최신 값으로 본다.\n\
- regime은 지금 움직임을 누가 끌고 있는지로 고른다. spot_led_rally는 현물과 ETF 매수가 끄는 상승, \
leverage_led_rally는 OI와 펀딩이 같이 오르는 상승, short_squeeze는 숏 청산이 끄는 급등, \
spot_led_selloff는 현물 매도와 ETF 유출이 끄는 하락, long_liquidation_cascade는 롱 청산이 이어지는 급락, \
macro_risk_off는 달러, 금리, 증시 같은 거시 변수가 끄는 하락, volatility_compression은 변동성과 거래가 줄어든 구간, \
event_wait는 큰 발표를 앞둔 관망, range_chop은 끄는 쪽 없이 오르내리는 구간이다.\n\
- bias_4h와 bias_24h는 4시간 뒤, 24시간 뒤 가격이 지금보다 높을지 낮을지다. 근거가 약하거나 엇갈리면 neutral.\n\
- confidence는 근거끼리 방향이 얼마나 맞는지와 데이터가 얼마나 차 있는지로 정한다. \
영향이 큰 일정이 2시간 안에 있으면 한 단계 낮춘다.\n\
- 직전 판정이 틀렸으면 같은 논리를 되풀이하지 말고 무엇이 달랐는지 반영한다.\n\
- 매수나 매도를 권하지 않는다.\n\
\n\
쓰는 법\n\
- 쓴 글은 화면에 그대로 나간다. 장중에 옆자리 동료에게 남기는 메모처럼 짧게 쓴다.\n\
- summary는 두 문장까지. 지금 무슨 일이 왜 일어나는지만 쓴다.\n\
- drivers는 4개까지이고 metric에는 입력 metrics의 key를 그대로 넣는다. 화면에서 note 앞에 지표 이름이 붙으니 \
note에는 이름을 다시 쓰지 말고 수치와 해석만 한 문장으로 쓴다.\n\
- conflicts에는 결론과 반대 방향인 신호를, invalidation에는 이 판정이 틀렸다고 볼 조건을 한 문장씩 쓴다.\n\
- 숫자는 화면 표기를 따른다. 달러 금액은 $4.7B, $350M, 퍼센트 단위 지표의 변화는 %p나 bp, z-score는 z +2.3으로 쓴다.\n\
- 이런 말투는 쓰지 않는다: ~로 판단됩니다, ~것으로 보입니다, ~할 필요가 있습니다, 주목할 만합니다, 시사합니다. \
가운뎃점으로 잇는 나열, 줄표, 괄호 안 영어 병기, 이모지도 쓰지 않는다.\n\
- 예시 note: 0.031%, 30일 평균의 세 배쯤이고 z +2.8. 롱이 한쪽으로 몰려 있다.\n\
- 예시 summary: ETF 유출이 사흘째인데 파생 쪽은 조용하다. 현물 매도가 하락을 끄는 모양새다.";

pub fn spawn(app: &App) {
    app.status.expect("judge", 3600);
    app.status.expect("ledger", 300);
    let a = app.clone();
    tokio::spawn(async move {
        // Polled against the wall clock: tokio timers stop while a Mac sleeps, so one long sleep
        // until the next hour would fire late by however long the lid was closed.
        let mut tick = interval(Duration::from_secs(30));
        let mut attempted = None;
        loop {
            tick.tick().await;
            let t = now();
            let hour = floor_to(t, HOUR);
            // Once per clock hour, 20s in so the minute collectors have fresh prices.
            if attempted == Some(hour) || t - hour < 20 {
                continue;
            }
            attempted = Some(hour);
            for asset in Asset::ALL {
                match judge(&a, asset).await {
                    Ok(()) => a.status.ok("judge"),
                    Err(e) => {
                        tracing::warn!("judge {}: {e:#}", asset.key());
                        a.status.err("judge", format!("{e:#}"));
                    }
                }
            }
        }
    });

    let a = app.clone();
    tokio::spawn(async move {
        let mut tick = interval(Duration::from_secs(300));
        loop {
            tick.tick().await;
            match score_due(&a.store, now()) {
                Ok(()) => a.status.ok("ledger"),
                Err(e) => {
                    tracing::warn!("ledger: {e:#}");
                    a.status.err("ledger", format!("{e:#}"));
                }
            }
        }
    });
}

/// At most one judgment per source, asset and clock hour (restarts do not duplicate).
async fn judge(app: &App, asset: Asset) -> Result<()> {
    let now = now();
    let hour = floor_to(now, HOUR);
    let snap = snapshot::build(&app.store, asset, now, app.whale_enabled())?;
    let price = snap.price.context("no fresh price")?;
    let events = app.store.events(now - HOUR, now + 24 * HOUR)?;
    let done = |source: &str| -> Result<bool> {
        Ok(app
            .store
            .judgments(asset.key(), Some(source), 1)?
            .first()
            .is_some_and(|j| j.ts >= hour))
    };
    let publish = |verdict: &Verdict| -> Result<()> {
        let judgment = app
            .store
            .insert_judgment(now, asset.key(), price, verdict)?;
        let _ = app.bus.send(BusEvent::Judgment(judgment));
        Ok(())
    };

    if !done("rules")? {
        publish(&rules_verdict(&snap, &events, now))?;
    }
    if let Some(claude) = &app.claude
        && !done("claude")?
    {
        match claude_verdict(app, claude, &snap, &events, now).await {
            Ok(verdict) => {
                publish(&verdict)?;
                app.status.ok("judge.claude");
            }
            Err(e) => {
                tracing::warn!("judge.claude {}: {e:#}", asset.key());
                app.status.err("judge.claude", format!("{e:#}"));
            }
        }
    }
    Ok(())
}

// ---- rule-based control ----

fn bias(score: Option<f64>) -> &'static str {
    match score {
        Some(s) if s >= 40.0 => "strong_up",
        Some(s) if s >= 15.0 => "up",
        Some(s) if s <= -40.0 => "strong_down",
        Some(s) if s <= -15.0 => "down",
        _ => "neutral",
    }
}

/// Deterministic judgment from the heuristic scores; the baseline Claude has to beat.
pub fn rules_verdict(s: &Snapshot, events: &[Event], now: i64) -> Verdict {
    let z = |key: &str| s.get(key).and_then(|m| m.z);
    let px_1h = s.chg_1h_pct.unwrap_or(0.0);
    let px_24h = s.chg_24h_pct.unwrap_or(0.0);
    let oi_24h = s
        .get("{a}.oi_usd")
        .and_then(|m| m.chg_24h_pct)
        .unwrap_or(0.0);
    let event_soon = events
        .iter()
        .any(|e| e.impact == "high" && e.ts > now && e.ts - now <= 2 * HOUR);

    let regime = if event_soon {
        "event_wait"
    } else if z("{a}.liq_long_usd").is_some_and(|z| z >= 3.0) && px_1h <= -1.0 {
        "long_liquidation_cascade"
    } else if z("{a}.liq_short_usd").is_some_and(|z| z >= 3.0) && px_1h >= 1.0 {
        "short_squeeze"
    } else if s.scores.macro_.is_some_and(|m| m <= -40.0) && px_24h < 0.0 {
        "macro_risk_off"
    } else if px_24h >= 1.5 {
        if oi_24h >= 5.0 && z("{a}.funding_avg").is_some_and(|z| z >= 1.0) {
            "leverage_led_rally"
        } else {
            "spot_led_rally"
        }
    } else if px_24h <= -1.5 {
        "spot_led_selloff"
    } else if z("{a}.dvol").is_some_and(|z| z <= -1.0) && px_24h.abs() < 1.0 {
        "volatility_compression"
    } else {
        "range_chop"
    };

    // The short horizon leans on positioning and flows; the day view uses every category.
    let short = match (s.scores.derivs, s.scores.flow) {
        (Some(d), Some(f)) => Some(0.6 * d + 0.4 * f),
        (d, f) => d.or(f).or(s.scores.total),
    };
    let bias_24h = bias(s.scores.total);

    let mut ranked: Vec<(&MetricView, f64)> = s
        .metrics
        .iter()
        .filter_map(|m| Some((m, m.pull()? * m.weight)))
        .filter(|(_, pull)| pull.abs() >= 0.5)
        .collect();
    ranked.sort_by(|a, b| b.1.abs().total_cmp(&a.1.abs()));
    // Driver notes sit next to the metric label in the UI, so they carry only the reading.
    let reading =
        |m: &MetricView| format!("{}{}", m.display(), m.z.map_or(String::new(), |z| format!(" (z {z:+.1})")));
    let drivers = ranked
        .iter()
        .take(3)
        .map(|(m, pull)| Driver {
            metric: m.key.clone(),
            direction: if *pull > 0.0 { "bullish" } else { "bearish" }.into(),
            note: reading(m),
        })
        .collect();
    let lean = s.scores.total.unwrap_or(0.0);
    let against: Vec<String> = ranked
        .iter()
        .filter(|(_, pull)| pull * lean < 0.0)
        .take(2)
        .map(|(m, _)| format!("{} {}", m.label, reading(m)))
        .collect();

    let covered = [
        s.scores.flow,
        s.scores.derivs,
        s.scores.macro_,
        s.scores.onchain,
    ]
    .iter()
    .flatten()
    .count();
    let mut level: usize = match lean.abs() {
        v if v >= 50.0 => 2,
        v if v >= 25.0 => 1,
        _ => 0,
    };
    if event_soon || covered < 3 {
        level = level.saturating_sub(1);
    }

    let fmt = |v: Option<f64>| v.map_or("없음".to_owned(), |v| format!("{v:+.0}"));
    Verdict {
        source: "rules",
        model: "rules-v1".into(),
        regime: regime.into(),
        bias_4h: bias(short).into(),
        bias_24h: bias_24h.into(),
        confidence: CONFIDENCES[level].into(),
        summary: format!(
            "종합 점수 {} (자금 {}, 파생 {}, 거시 {}, 온체인 {}). 24시간 가격 {}.",
            fmt(s.scores.total),
            fmt(s.scores.flow),
            fmt(s.scores.derivs),
            fmt(s.scores.macro_),
            fmt(s.scores.onchain),
            s.chg_24h_pct.map_or("없음".to_owned(), |c| format!("{c:+.2}%")),
        ),
        drivers,
        conflicts: if against.is_empty() { "없음".into() } else { against.join(", ") },
        invalidation: match bias_24h {
            "up" | "strong_up" => "종합 점수가 0 밑으로 내려가거나 가격이 1.5% 넘게 빠지면 무효",
            "down" | "strong_down" => "종합 점수가 0 위로 올라오거나 가격이 1.5% 넘게 오르면 무효",
            _ => "종합 점수가 +15나 -15를 넘으면 다시 판정",
        }
        .into(),
        tokens: None,
    }
}

// ---- Claude ----

fn judge_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "regime": { "type": "string", "enum": REGIMES },
            "bias_4h": { "type": "string", "enum": BIASES },
            "bias_24h": { "type": "string", "enum": BIASES },
            "confidence": { "type": "string", "enum": CONFIDENCES },
            "summary": { "type": "string" },
            "drivers": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "metric": { "type": "string" },
                        "direction": { "type": "string", "enum": DIRECTIONS },
                        "note": { "type": "string" }
                    },
                    "required": ["metric", "direction", "note"],
                    "additionalProperties": false
                }
            },
            "conflicts": { "type": "string" },
            "invalidation": { "type": "string" }
        },
        "required": ["regime", "bias_4h", "bias_24h", "confidence", "summary", "drivers", "conflicts", "invalidation"],
        "additionalProperties": false
    })
}

/// Five significant digits: enough for the model, far fewer tokens than raw floats.
fn sig(v: Option<f64>) -> Value {
    match v.filter(|x| x.is_finite()) {
        None => Value::Null,
        Some(x) if x == 0.0 => json!(0.0),
        Some(x) => {
            let scale = 10f64.powi(4 - x.abs().log10().floor() as i32);
            json!((x * scale).round() / scale)
        }
    }
}

fn judge_input(app: &App, s: &Snapshot, events: &[Event], now: i64) -> Result<Value> {
    let metrics: Vec<Value> = s
        .metrics
        .iter()
        .map(|m| {
            json!({
                "key": m.key,
                "label": m.label,
                "category": m.category,
                // "coin" is an internal unit key; the model would copy it into its prose.
                "unit": if m.unit == "coin" { s.asset.sym() } else { m.unit },
                "value": sig(m.value),
                "cadence": m.cadence,
                "stale": m.stale,
                "chg_1h": sig(m.chg_1h),
                "chg_24h": sig(m.chg_24h),
                "z": sig(m.z),
                "signal": m.signal,
            })
        })
        .collect();
    let upcoming: Vec<Value> = events
        .iter()
        .filter(|e| e.ts > now)
        .map(|e| {
            json!({
                "in_minutes": (e.ts - now) / 60,
                "title": e.title,
                "impact": e.impact,
                "forecast": e.forecast,
                "previous": e.previous,
            })
        })
        .collect();
    let news: Vec<Value> = app
        .store
        .news_since(now - 2 * HOUR, 15)?
        .iter()
        .map(|n| json!({ "minutes_ago": (now - n.ts) / 60, "title": n.title, "important": n.important, "tag": n.tag }))
        .collect();
    let prior: Vec<Value> = app
        .store
        .judgments(s.asset.key(), Some("claude"), 6)?
        .iter()
        .map(|j| {
            json!({
                "hours_ago": (now - j.ts) / HOUR,
                "regime": j.regime,
                "bias_4h": j.bias_4h,
                "bias_24h": j.bias_24h,
                "confidence": j.confidence,
                "ret_4h_pct": sig(j.ret_4h_pct),
                "hit_4h": j.hit_4h,
                "ret_24h_pct": sig(j.ret_24h_pct),
                "hit_24h": j.hit_24h,
            })
        })
        .collect();
    Ok(json!({
        "asset": s.asset.sym(),
        "time_kst": kst(now).format("%Y-%m-%d %H:%M").to_string(),
        "price_usd": sig(s.price),
        "price_chg_1h_pct": sig(s.chg_1h_pct),
        "price_chg_24h_pct": sig(s.chg_24h_pct),
        "scores": s.scores,
        "etf_streak_days": s.etf_streak,
        "metrics": metrics,
        "upcoming_events_24h": upcoming,
        "recent_news_2h": news,
        "prior_judgments": prior,
    }))
}

async fn claude_verdict(
    app: &App,
    claude: &Claude,
    s: &Snapshot,
    events: &[Event],
    now: i64,
) -> Result<Verdict> {
    let input = judge_input(app, s, events, now)?;
    // Thinking is always on and counts toward max_tokens, so the cap leaves room for it.
    let reply = claude
        .structured(&app.cfg.claude_model, JUDGE_SYSTEM, &input.to_string(), judge_schema(), 16_000)
        .await?;
    let v = &reply.json;
    // Enum casing is not guaranteed by structured outputs.
    let pick = |field: &str, allowed: &[&str]| -> Result<String> {
        let value = v[field]
            .as_str()
            .map(str::to_lowercase)
            .with_context(|| format!("claude: {field} missing"))?;
        if !allowed.contains(&value.as_str()) {
            bail!("claude: unexpected {field} {value:?}");
        }
        Ok(value)
    };
    let text = |field: &str| v[field].as_str().unwrap_or_default().trim().to_owned();
    let drivers = v["drivers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|d| {
            Some(Driver {
                metric: d["metric"].as_str()?.into(),
                direction: d["direction"]
                    .as_str()
                    .map(str::to_lowercase)
                    .filter(|x| DIRECTIONS.contains(&x.as_str()))?,
                note: d["note"].as_str()?.into(),
            })
        })
        .take(4)
        .collect();
    Ok(Verdict {
        source: "claude",
        model: app.cfg.claude_model.clone(),
        regime: pick("regime", &REGIMES)?,
        bias_4h: pick("bias_4h", &BIASES)?,
        bias_24h: pick("bias_24h", &BIASES)?,
        confidence: pick("confidence", &CONFIDENCES)?,
        summary: text("summary"),
        drivers,
        conflicts: text("conflicts"),
        invalidation: text("invalidation"),
        tokens: Some((reply.input_tokens, reply.output_tokens)),
    })
}

// ---- ledger ----

/// A neutral call is right when the price stayed inside this band (percent).
fn neutral_band(h: Horizon) -> f64 {
    match h {
        Horizon::H4 => 0.5,
        Horizon::H24 => 1.5,
    }
}

fn directional(bias: &str) -> Option<f64> {
    match bias {
        "up" | "strong_up" => Some(1.0),
        "down" | "strong_down" => Some(-1.0),
        _ => None,
    }
}

pub fn hit(bias: &str, ret_pct: f64, h: Horizon) -> bool {
    match directional(bias) {
        Some(side) => ret_pct * side > 0.0,
        None => ret_pct.abs() < neutral_band(h),
    }
}

/// Fills in realised returns for judgments whose horizon has passed.
pub fn score_due(store: &Store, now: i64) -> Result<()> {
    for h in [Horizon::H4, Horizon::H24] {
        for j in store.unscored(h, now)? {
            let Some(asset) = Asset::parse(&j.asset) else {
                continue;
            };
            // No price near the horizon (collector gap): leave unscored rather than guess.
            let Some((_, later)) =
                store.at_or_before(&asset.m("price"), j.ts + h.secs(), 15 * 60)?
            else {
                continue;
            };
            let Some(ret) = pct(later, j.price) else {
                continue;
            };
            let bias = match h {
                Horizon::H4 => &j.bias_4h,
                Horizon::H24 => &j.bias_24h,
            };
            store.set_outcome(j.id, h, ret, hit(bias, ret, h))?;
        }
    }
    Ok(())
}

#[derive(Serialize, Default, Debug, PartialEq)]
pub struct HorizonStats {
    pub n: usize,
    pub directional_n: usize,
    pub hits: usize,
    pub hit_rate: Option<f64>,
    pub avg_ret_up_pct: Option<f64>,
    pub avg_ret_down_pct: Option<f64>,
}

#[derive(Serialize)]
pub struct SourceStats {
    pub source: &'static str,
    pub label: &'static str,
    pub h4: HorizonStats,
    pub h24: HorizonStats,
}

#[derive(Serialize)]
pub struct RegimeStats {
    pub source: &'static str,
    pub regime: String,
    pub n: usize,
    pub avg_ret_4h_pct: Option<f64>,
    pub avg_ret_24h_pct: Option<f64>,
}

#[derive(Serialize)]
pub struct Ledger {
    pub asset: &'static str,
    pub sources: Vec<SourceStats>,
    pub by_regime: Vec<RegimeStats>,
}

fn mean(values: &[f64]) -> Option<f64> {
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}

/// `calls`: (bias, realised return in percent).
fn stats(calls: &[(&str, f64)]) -> HorizonStats {
    let (mut up, mut down, mut hits) = (Vec::new(), Vec::new(), 0);
    for &(bias, ret) in calls {
        match directional(bias) {
            Some(side) if side > 0.0 => up.push(ret),
            Some(_) => down.push(ret),
            None => continue,
        }
        hits += usize::from(ret * directional(bias).unwrap_or(0.0) > 0.0);
    }
    let directional_n = up.len() + down.len();
    HorizonStats {
        n: calls.len(),
        directional_n,
        hits,
        hit_rate: (directional_n > 0).then(|| hits as f64 / directional_n as f64),
        avg_ret_up_pct: mean(&up),
        avg_ret_down_pct: mean(&down),
    }
}

fn ret_of(j: &Judgment, h: Horizon) -> Option<f64> {
    match h {
        Horizon::H4 => j.ret_4h_pct,
        Horizon::H24 => j.ret_24h_pct,
    }
}

fn bias_of(j: &Judgment, h: Horizon) -> &str {
    match h {
        Horizon::H4 => &j.bias_4h,
        Horizon::H24 => &j.bias_24h,
    }
}

pub fn ledger(store: &Store, asset: Asset) -> Result<Ledger> {
    let all = store.judgments(asset.key(), None, 4000)?;
    let of =
        |source: &str| -> Vec<&Judgment> { all.iter().filter(|j| j.source == source).collect() };
    let scored = |js: &[&Judgment], h: Horizon| -> HorizonStats {
        let calls: Vec<(&str, f64)> = js
            .iter()
            .filter_map(|j| Some((bias_of(j, h), ret_of(j, h)?)))
            .collect();
        stats(&calls)
    };

    let mut sources = Vec::new();
    let mut by_regime = Vec::new();
    for (source, label) in [("claude", "Claude"), ("rules", "룰")] {
        let js = of(source);
        sources.push(SourceStats {
            source,
            label,
            h4: scored(&js, Horizon::H4),
            h24: scored(&js, Horizon::H24),
        });

        let mut regimes: Vec<&str> = js.iter().map(|j| j.regime.as_str()).collect();
        regimes.sort_unstable();
        regimes.dedup();
        for regime in regimes {
            let rows: Vec<&&Judgment> = js.iter().filter(|j| j.regime == regime).collect();
            let rets =
                |h: Horizon| -> Vec<f64> { rows.iter().filter_map(|j| ret_of(j, h)).collect() };
            by_regime.push(RegimeStats {
                source,
                regime: regime.into(),
                n: rows.len(),
                avg_ret_4h_pct: mean(&rets(Horizon::H4)),
                avg_ret_24h_pct: mean(&rets(Horizon::H24)),
            });
        }
    }

    // Naive baselines over the same hourly timestamps as the rule judgments.
    let hourly = of("rules");
    let price_key = asset.m("price");
    let baseline = |h: Horizon, momentum: bool| -> Result<HorizonStats> {
        let mut calls = Vec::new();
        for j in &hourly {
            let Some(ret) = ret_of(j, h) else { continue };
            let bias = if momentum {
                // Follow the move over the preceding window of the same length.
                let Some((_, before)) = store.at_or_before(&price_key, j.ts - h.secs(), 15 * 60)?
                else {
                    continue;
                };
                if j.price > before {
                    "up"
                } else if j.price < before {
                    "down"
                } else {
                    "neutral"
                }
            } else {
                "up"
            };
            calls.push((bias, ret));
        }
        Ok(stats(&calls))
    };
    for (source, label, momentum) in [
        ("baseline_always_up", "기준선 (항상 상승)", false),
        ("baseline_momentum", "기준선 (직전 추세)", true),
    ] {
        sources.push(SourceStats {
            source,
            label,
            h4: baseline(Horizon::H4, momentum)?,
            h24: baseline(Horizon::H24, momentum)?,
        });
    }
    Ok(Ledger {
        asset: asset.key(),
        sources,
        by_regime,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::DAY;

    fn verdict(source: &'static str, bias_4h: &str, bias_24h: &str) -> Verdict {
        Verdict {
            source,
            model: "test".into(),
            regime: "range_chop".into(),
            bias_4h: bias_4h.into(),
            bias_24h: bias_24h.into(),
            confidence: "low".into(),
            summary: String::new(),
            drivers: Vec::new(),
            conflicts: String::new(),
            invalidation: String::new(),
            tokens: None,
        }
    }

    #[test]
    fn hit_rules_for_directional_and_neutral_calls() {
        assert!(hit("up", 0.1, Horizon::H4));
        assert!(!hit("strong_up", -0.1, Horizon::H4));
        assert!(hit("down", -2.0, Horizon::H24));
        // Neutral is right only inside the band, which is wider for the longer horizon.
        assert!(hit("neutral", 0.4, Horizon::H4));
        assert!(!hit("neutral", -0.6, Horizon::H4));
        assert!(hit("neutral", -1.4, Horizon::H24));
    }

    #[test]
    fn scoring_waits_for_the_horizon_and_uses_the_price_at_it() {
        let store = Store::open(":memory:").unwrap();
        let t0 = 100 * DAY;
        store
            .insert_judgment(t0, "btc", 100.0, &verdict("rules", "up", "down"))
            .unwrap();
        store
            .put_many(
                "btc.price",
                &[
                    (t0 + 4 * HOUR - 60, 102.0),
                    (t0 + 5 * HOUR, 90.0),
                    (t0 + 24 * HOUR, 97.0),
                ],
            )
            .unwrap();

        score_due(&store, t0 + 3 * HOUR).unwrap();
        assert_eq!(store.judgments("btc", None, 1).unwrap()[0].ret_4h_pct, None);

        score_due(&store, t0 + 6 * HOUR).unwrap();
        let j = &store.judgments("btc", None, 1).unwrap()[0];
        // The later, lower print must not leak into the 4h outcome.
        assert!((j.ret_4h_pct.unwrap() - 2.0).abs() < 1e-9);
        assert_eq!((j.hit_4h, j.ret_24h_pct), (Some(true), None));

        score_due(&store, t0 + 25 * HOUR).unwrap();
        let j = &store.judgments("btc", None, 1).unwrap()[0];
        assert!((j.ret_24h_pct.unwrap() + 3.0).abs() < 1e-9);
        assert_eq!(j.hit_24h, Some(true));
    }

    #[test]
    fn scoring_skips_judgments_without_a_price_near_the_horizon() {
        let store = Store::open(":memory:").unwrap();
        let t0 = 100 * DAY;
        store
            .insert_judgment(t0, "btc", 100.0, &verdict("rules", "up", "up"))
            .unwrap();
        store.put("btc.price", t0 + 3 * HOUR, 120.0).unwrap();
        score_due(&store, t0 + 5 * HOUR).unwrap();
        assert_eq!(store.judgments("btc", None, 1).unwrap()[0].ret_4h_pct, None);
    }

    #[test]
    fn stats_exclude_neutral_calls_from_the_hit_rate() {
        let s = stats(&[
            ("up", 1.0),
            ("strong_up", -0.5),
            ("down", -2.0),
            ("neutral", 0.1),
        ]);
        assert_eq!((s.n, s.directional_n, s.hits), (4, 3, 2));
        assert!((s.hit_rate.unwrap() - 2.0 / 3.0).abs() < 1e-12);
        assert_eq!(
            (s.avg_ret_up_pct, s.avg_ret_down_pct),
            (Some(0.25), Some(-2.0))
        );
        assert_eq!(stats(&[("neutral", 0.1)]).hit_rate, None);
    }

    #[test]
    fn ledger_compares_sources_with_baselines_on_the_same_hours() {
        let store = Store::open(":memory:").unwrap();
        let t0 = 100 * DAY;
        // Price falls for 8 hours: 100 -> 96 -> 92.
        store
            .put_many(
                "btc.price",
                &[(t0 - 4 * HOUR, 100.0), (t0, 96.0), (t0 + 4 * HOUR, 92.0)],
            )
            .unwrap();
        store
            .insert_judgment(t0, "btc", 96.0, &verdict("rules", "down", "neutral"))
            .unwrap();
        store
            .insert_judgment(t0, "btc", 96.0, &verdict("claude", "up", "neutral"))
            .unwrap();
        score_due(&store, t0 + 5 * HOUR).unwrap();

        let l = ledger(&store, Asset::Btc).unwrap();
        let rate = |source: &str| {
            l.sources
                .iter()
                .find(|s| s.source == source)
                .unwrap()
                .h4
                .hit_rate
        };
        assert_eq!(rate("rules"), Some(1.0));
        assert_eq!(rate("claude"), Some(0.0));
        assert_eq!(rate("baseline_always_up"), Some(0.0));
        assert_eq!(rate("baseline_momentum"), Some(1.0));
        assert_eq!(l.by_regime.len(), 2);
    }

    #[test]
    fn rules_verdict_stands_down_before_a_high_impact_event() {
        let store = Store::open(":memory:").unwrap();
        let now = 100 * DAY;
        let snap = snapshot::build(&store, Asset::Btc, now, false).unwrap();
        let event = Event {
            id: "e".into(),
            ts: now + HOUR,
            title: "CPI m/m".into(),
            kind: "macro".into(),
            impact: "high".into(),
            forecast: None,
            previous: None,
        };
        let v = rules_verdict(&snap, &[event], now);
        assert_eq!(
            (
                v.regime.as_str(),
                v.bias_24h.as_str(),
                v.confidence.as_str()
            ),
            ("event_wait", "neutral", "low")
        );
        assert_eq!(rules_verdict(&snap, &[], now).regime, "range_chop");
    }
}
