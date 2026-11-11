//! The metrics shown in the app and how each one feeds the heuristic score.

use serde::Serialize;

use crate::model::Asset;

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Flow,
    Derivs,
    Macro,
    Onchain,
}

impl Category {
    /// Weight of the category in the total score.
    pub fn weight(self) -> f64 {
        match self {
            Category::Flow | Category::Derivs => 0.3,
            Category::Macro | Category::Onchain => 0.2,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Cadence {
    Intraday,
    Daily,
    Weekly,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The stored points are the value.
    Level,
    /// The stored points are per-minute amounts; the value is their trailing one-hour sum.
    Sum1h,
}

/// What the z-score is computed on.
#[derive(Clone, Copy)]
pub enum Basis {
    /// The level itself (mean-reverting series such as funding or premiums).
    Level,
    /// The 24h percentage change (trending series). The payload is a typical one-sigma move,
    /// used as the scale until enough history exists for a real z-score.
    ChgPct(f64),
    /// The 24h absolute change, with the same fallback scale.
    ChgAbs(f64),
}

pub struct Def {
    /// Storage key; `{a}` is replaced by the asset (`btc` / `eth`).
    pub key: &'static str,
    pub label: &'static str,
    pub category: Category,
    pub unit: &'static str,
    pub decimals: u8,
    pub cadence: Cadence,
    pub kind: Kind,
    pub basis: Basis,
    /// +1: higher is bullish, -1: higher is bearish, 0: shown but not scored.
    pub dir: f64,
    pub weight: f64,
    /// One short line under the label: where the number comes from and how to read it.
    pub note: &'static str,
    /// Only populated when a Whale Alert key is configured.
    pub whale: bool,
}

impl Def {
    pub fn key_for(&self, asset: Asset) -> String {
        self.key.replace("{a}", asset.key())
    }
}

/// Catalog entry for a concrete storage key such as `eth.funding_avg`.
pub fn find(key: &str) -> Option<&'static Def> {
    DEFS.iter().find(|d| Asset::ALL.iter().any(|a| d.key_for(*a) == key))
}

const BASE: Def = Def {
    key: "",
    label: "",
    category: Category::Flow,
    unit: "",
    decimals: 2,
    cadence: Cadence::Intraday,
    kind: Kind::Level,
    basis: Basis::Level,
    dir: 0.0,
    weight: 0.0,
    note: "",
    whale: false,
};
const FLOW: Def = Def { category: Category::Flow, ..BASE };
const DERIVS: Def = Def { category: Category::Derivs, ..BASE };
const MACRO: Def = Def { category: Category::Macro, ..BASE };
const ONCHAIN: Def = Def { category: Category::Onchain, ..BASE };

pub const DEFS: &[Def] = &[
    // ---- 자금 흐름 ----
    Def {
        key: "etf.{a}.netflow_usd",
        label: "현물 ETF 순유입",
        unit: "$",
        decimals: 0,
        cadence: Cadence::Daily,
        dir: 1.0,
        weight: 1.5,
        note: "미국 장 마감 뒤 집계, 한국 오전에 확정",
        ..FLOW
    },
    Def {
        key: "{a}.coinbase_premium_pct",
        label: "Coinbase 프리미엄",
        unit: "%",
        decimals: 3,
        dir: 1.0,
        weight: 1.0,
        note: "Coinbase와 Binance 가격 차. 양수면 미국 쪽 매수가 강함",
        ..FLOW
    },
    Def {
        key: "{a}.kimchi_premium_pct",
        label: "김치 프리미엄",
        unit: "%",
        dir: -1.0,
        weight: 0.5,
        note: "Upbit와 해외 가격 차. 급하게 오르면 국내 과열",
        ..FLOW
    },
    Def {
        key: "stable.net_7d_usd",
        label: "스테이블코인 7일 순발행",
        unit: "$",
        decimals: 0,
        cadence: Cadence::Daily,
        dir: 1.0,
        weight: 1.0,
        note: "USDT와 USDC 유통량이 7일 동안 늘어난 양",
        ..FLOW
    },
    Def {
        key: "stable.net_1d_usd",
        label: "스테이블코인 하루 순발행",
        unit: "$",
        decimals: 0,
        cadence: Cadence::Daily,
        note: "USDT와 USDC 유통량의 전일 대비 증감",
        ..FLOW
    },
    Def {
        key: "{a}.cme_basis_annual_pct",
        label: "CME 베이시스 (연율)",
        unit: "%",
        note: "CME 근월물과 현물 가격 차. 지연 시세라 시간 단위로는 많이 튐",
        ..FLOW
    },
    // ---- 파생 ----
    Def {
        key: "{a}.funding_avg",
        label: "펀딩비 (8시간)",
        unit: "%",
        decimals: 4,
        dir: -1.0,
        weight: 1.5,
        note: "Binance와 Bybit 평균. 높으면 롱 과열, 음수면 숏 과열",
        ..DERIVS
    },
    Def {
        key: "{a}.oi_usd",
        label: "미결제약정",
        unit: "$",
        decimals: 0,
        basis: Basis::ChgPct(4.0),
        note: "Binance 기준. 가격은 그대로인데 OI만 늘면 레버리지가 쌓이는 중",
        ..DERIVS
    },
    Def {
        key: "{a}.basis_q_annual_pct",
        label: "분기물 베이시스 (연율)",
        unit: "%",
        dir: -1.0,
        weight: 0.5,
        note: "Binance 분기 선물과 현물 가격 차",
        ..DERIVS
    },
    Def {
        key: "{a}.liq_long_usd",
        label: "롱 청산 (1시간)",
        unit: "$",
        decimals: 0,
        kind: Kind::Sum1h,
        note: "Binance, Bybit, OKX 합계",
        ..DERIVS
    },
    Def {
        key: "{a}.liq_short_usd",
        label: "숏 청산 (1시간)",
        unit: "$",
        decimals: 0,
        kind: Kind::Sum1h,
        note: "Binance, Bybit, OKX 합계",
        ..DERIVS
    },
    Def {
        key: "{a}.dvol",
        label: "DVOL",
        unit: "pt",
        decimals: 1,
        note: "Deribit 30일 내재변동성",
        ..DERIVS
    },
    Def {
        key: "{a}.skew25d",
        label: "25델타 스큐",
        unit: "pt",
        dir: -1.0,
        weight: 0.5,
        note: "풋 IV에서 콜 IV를 뺀 값. 양수면 하락 대비 수요가 많음",
        ..DERIVS
    },
    // ---- 거시 ----
    Def {
        key: "macro.dxy",
        label: "달러 인덱스",
        basis: Basis::ChgPct(0.4),
        dir: -1.0,
        weight: 1.0,
        note: "달러가 오르면 코인에는 부담",
        ..MACRO
    },
    Def {
        key: "macro.us10y",
        label: "미 10년물 금리",
        unit: "%",
        decimals: 3,
        basis: Basis::ChgAbs(0.06),
        dir: -1.0,
        weight: 1.0,
        ..MACRO
    },
    Def {
        key: "macro.us2y",
        label: "미 2년물 금리",
        unit: "%",
        decimals: 3,
        basis: Basis::ChgAbs(0.06),
        dir: -1.0,
        weight: 0.5,
        ..MACRO
    },
    Def {
        key: "macro.nq",
        label: "나스닥100 선물",
        decimals: 0,
        basis: Basis::ChgPct(1.1),
        dir: 1.0,
        weight: 1.0,
        note: "근월물, 지연 시세",
        ..MACRO
    },
    Def {
        key: "macro.vix",
        label: "VIX",
        basis: Basis::ChgPct(7.0),
        dir: -1.0,
        weight: 1.0,
        note: "미국 정규장에만 움직임",
        ..MACRO
    },
    Def {
        key: "macro.usdjpy",
        label: "USD/JPY",
        basis: Basis::ChgPct(0.6),
        dir: 1.0,
        weight: 0.5,
        note: "급락하면 엔캐리 청산 걱정",
        ..MACRO
    },
    Def {
        key: "macro.fed.expected_bp",
        label: "FOMC 기대 변화폭",
        unit: "bp",
        decimals: 1,
        basis: Basis::ChgAbs(5.0),
        dir: -1.0,
        weight: 1.0,
        note: "Kalshi 금리 시장 기준, 다음 회의. 양수면 인상 쪽",
        ..MACRO
    },
    Def {
        key: "macro.fed.p_cut",
        label: "FOMC 인하 확률",
        unit: "%",
        decimals: 1,
        basis: Basis::ChgAbs(5.0),
        note: "Kalshi 기준, 다음 회의",
        ..MACRO
    },
    Def {
        key: "macro.fed.p_hike",
        label: "FOMC 인상 확률",
        unit: "%",
        decimals: 1,
        basis: Basis::ChgAbs(5.0),
        note: "Kalshi 기준, 다음 회의",
        ..MACRO
    },
    Def {
        key: "macro.net_liquidity_usd",
        label: "연준 순유동성",
        unit: "$",
        decimals: 0,
        cadence: Cadence::Weekly,
        note: "대차대조표에서 TGA와 RRP를 뺀 값. 주 1회, 변화는 전주 대비",
        ..MACRO
    },
    // ---- 온체인 ----
    Def {
        key: "{a}.ex_netflow",
        label: "거래소 순입금",
        unit: "coin",
        decimals: 0,
        cadence: Cadence::Daily,
        dir: -1.0,
        weight: 1.5,
        note: "Coin Metrics 일간. 입금이 몰리면 팔 물량이 대기 중",
        ..ONCHAIN
    },
    Def {
        key: "{a}.ex_supply",
        label: "거래소 보유량",
        unit: "coin",
        decimals: 0,
        cadence: Cadence::Daily,
        basis: Basis::ChgPct(0.3),
        note: "Coin Metrics 일간",
        ..ONCHAIN
    },
    Def {
        key: "cex.inflow_24h_usd",
        label: "거래소 24시간 순유입",
        unit: "$",
        decimals: 0,
        dir: -1.0,
        weight: 0.5,
        note: "DefiLlama, 코인 구분 없이 전체",
        ..ONCHAIN
    },
    Def {
        key: "{a}.whale_to_ex_usd",
        label: "고래 거래소 입금 (1시간)",
        unit: "$",
        decimals: 0,
        kind: Kind::Sum1h,
        note: "Whale Alert, 2천만 달러 이상 이체",
        whale: true,
        ..ONCHAIN
    },
];
