use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Asset {
    Btc,
    Eth,
}

impl Asset {
    pub const ALL: [Asset; 2] = [Asset::Btc, Asset::Eth];

    pub fn key(self) -> &'static str {
        match self {
            Asset::Btc => "btc",
            Asset::Eth => "eth",
        }
    }

    pub fn sym(self) -> &'static str {
        match self {
            Asset::Btc => "BTC",
            Asset::Eth => "ETH",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "btc" => Some(Asset::Btc),
            "eth" => Some(Asset::Eth),
            _ => None,
        }
    }

    /// Metric key for a per-asset series, e.g. `btc.price`.
    pub fn m(self, name: &str) -> String {
        format!("{}.{name}", self.key())
    }
}

pub const REGIMES: [&str; 9] = [
    "spot_led_rally",
    "leverage_led_rally",
    "short_squeeze",
    "spot_led_selloff",
    "long_liquidation_cascade",
    "macro_risk_off",
    "volatility_compression",
    "event_wait",
    "range_chop",
];

pub fn regime_ko(regime: &str) -> &str {
    match regime {
        "spot_led_rally" => "현물 주도 상승",
        "leverage_led_rally" => "레버리지 주도 상승",
        "short_squeeze" => "숏 스퀴즈",
        "spot_led_selloff" => "현물 주도 하락",
        "long_liquidation_cascade" => "롱 청산 연쇄",
        "macro_risk_off" => "거시발 위험 회피",
        "volatility_compression" => "변동성 수축",
        "event_wait" => "이벤트 대기",
        "range_chop" => "방향 없음",
        other => other,
    }
}

pub fn bias_ko(bias: &str) -> &str {
    match bias {
        "strong_down" => "강한 하락",
        "down" => "하락",
        "neutral" => "중립",
        "up" => "상승",
        "strong_up" => "강한 상승",
        other => other,
    }
}

pub const BIASES: [&str; 5] = ["strong_down", "down", "neutral", "up", "strong_up"];
pub const CONFIDENCES: [&str; 3] = ["low", "mid", "high"];
pub const DIRECTIONS: [&str; 3] = ["bullish", "bearish", "neutral"];

#[derive(Clone, Serialize)]
pub struct Alert {
    pub id: i64,
    pub ts: i64,
    pub tier: String,
    pub severity: String,
    pub rule: String,
    pub asset: Option<String>,
    pub title: String,
    pub body: String,
}

#[derive(Clone, Serialize)]
pub struct Event {
    pub id: String,
    pub ts: i64,
    pub title: String,
    pub kind: String,
    pub impact: String,
    pub forecast: Option<String>,
    pub previous: Option<String>,
}

#[derive(Clone, Serialize)]
pub struct NewsTag {
    pub asset: String,
    pub direction: String,
    pub importance: i64,
}

#[derive(Clone, Serialize)]
pub struct NewsItem {
    pub id: i64,
    pub ts: i64,
    pub title: String,
    pub content: String,
    pub link: Option<String>,
    pub important: bool,
    pub bull: i64,
    pub bear: i64,
    pub tag: Option<NewsTag>,
    pub react_5m_pct: Option<f64>,
    pub react_1h_pct: Option<f64>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Driver {
    pub metric: String,
    pub direction: String,
    pub note: String,
}

#[derive(Clone, Serialize)]
pub struct Judgment {
    pub id: i64,
    pub ts: i64,
    pub asset: String,
    pub source: String,
    pub model: String,
    pub regime: String,
    pub bias_4h: String,
    pub bias_24h: String,
    pub confidence: String,
    pub summary: String,
    pub drivers: Vec<Driver>,
    pub conflicts: String,
    pub invalidation: String,
    pub price: f64,
    pub ret_4h_pct: Option<f64>,
    pub ret_24h_pct: Option<f64>,
    pub hit_4h: Option<bool>,
    pub hit_24h: Option<bool>,
}

/// A judgment before it is persisted.
pub struct Verdict {
    pub source: &'static str,
    pub model: String,
    pub regime: String,
    pub bias_4h: String,
    pub bias_24h: String,
    pub confidence: String,
    pub summary: String,
    pub drivers: Vec<Driver>,
    pub conflicts: String,
    pub invalidation: String,
    pub tokens: Option<(i64, i64)>,
}

#[derive(Clone, Serialize)]
pub struct Brief {
    pub ts: i64,
    pub body: String,
}
