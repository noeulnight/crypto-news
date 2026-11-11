// Types and fetch helpers mirroring the backend API contract (v1).
// Timestamps are unix seconds (UTC). Nullable fields are explicit `null`.

export type Asset = 'btc' | 'eth';
export const ASSETS: readonly Asset[] = ['btc', 'eth'];

export type Category = 'flow' | 'derivs' | 'macro' | 'onchain';
export type Unit = '%' | '$' | 'bp' | 'pt' | 'coin' | '';
export type Cadence = 'intraday' | 'daily' | 'weekly';
export type Signal = 'bullish' | 'bearish' | 'neutral';
/** `[ts, value]` */
export type Point = [number, number];

export interface Metric {
	key: string;
	label: string;
	category: Category;
	unit: Unit;
	decimals: number;
	cadence: Cadence;
	value: number | null;
	ts: number | null;
	chg_1h: number | null;
	chg_24h: number | null;
	z: number | null;
	signal: Signal;
	note: string;
	spark: Point[];
}

export type JudgmentSource = 'claude' | 'rules';
export type Regime =
	| 'spot_led_rally'
	| 'leverage_led_rally'
	| 'short_squeeze'
	| 'spot_led_selloff'
	| 'long_liquidation_cascade'
	| 'macro_risk_off'
	| 'volatility_compression'
	| 'event_wait'
	| 'range_chop';
export type Bias = 'strong_down' | 'down' | 'neutral' | 'up' | 'strong_up';
export type Confidence = 'low' | 'mid' | 'high';

export interface Driver {
	metric: string;
	direction: Signal;
	note: string;
}

export interface Judgment {
	id: number;
	ts: number;
	asset: Asset;
	source: JudgmentSource;
	model: string;
	regime: Regime;
	bias_4h: Bias;
	bias_24h: Bias;
	confidence: Confidence;
	summary: string;
	drivers: Driver[];
	conflicts: string;
	invalidation: string;
	price: number;
	ret_4h_pct: number | null;
	ret_24h_pct: number | null;
	hit_4h: boolean | null;
	hit_24h: boolean | null;
}

export type Scores = { total: number | null } & Record<Category, number | null>;

export interface Overview {
	asset: Asset;
	ts: number;
	price: number | null;
	chg_1h_pct: number | null;
	chg_24h_pct: number | null;
	claude_enabled: boolean;
	scores: Scores;
	judgments: Record<JudgmentSource, Judgment | null>;
	metrics: Metric[];
}

export type SeriesMode = 'raw' | 'z';

export interface Series {
	metric: string;
	mode: SeriesMode;
	points: Point[];
}

export interface NewsTag {
	asset: 'btc' | 'eth' | 'alt' | 'macro' | 'regulation' | 'other';
	direction: Signal;
	importance: number;
}

export interface NewsItem {
	id: number;
	ts: number;
	title: string;
	content: string;
	link: string | null;
	important: boolean;
	bull: number;
	bear: number;
	tag: NewsTag | null;
	react_5m_pct: number | null;
	react_1h_pct: number | null;
}

export type EventKind = 'macro' | 'fomc' | 'options_expiry';
export type EventImpact = 'high' | 'medium';

export interface EventItem {
	id: string;
	ts: number;
	title: string;
	kind: EventKind;
	impact: EventImpact;
	forecast: string | null;
	previous: string | null;
}

export type AlertTier = 'scheduled' | 'realtime' | 'composite';
export type Severity = 'info' | 'warn' | 'strong';

export interface Alert {
	id: number;
	ts: number;
	tier: AlertTier;
	severity: Severity;
	rule: string;
	asset: Asset | null;
	title: string;
	body: string;
}

export interface LedgerHorizon {
	n: number;
	directional_n: number;
	hits: number;
	hit_rate: number | null;
	avg_ret_up_pct: number | null;
	avg_ret_down_pct: number | null;
}

export type LedgerSourceKey = JudgmentSource | 'baseline_always_up' | 'baseline_momentum';

export interface LedgerSource {
	source: LedgerSourceKey;
	label: string;
	h4: LedgerHorizon;
	h24: LedgerHorizon;
}

export interface LedgerRegimeRow {
	source: LedgerSourceKey;
	regime: Regime;
	n: number;
	avg_ret_4h_pct: number | null;
	avg_ret_24h_pct: number | null;
}

export interface Ledger {
	asset: Asset;
	sources: LedgerSource[];
	by_regime: LedgerRegimeRow[];
}

export interface BriefEntry {
	ts: number;
	body: string;
}

export interface Brief {
	morning: BriefEntry | null;
	weekly: BriefEntry | null;
}

export interface Collector {
	name: string;
	last_ok_ts: number | null;
	last_err: string | null;
	last_err_ts: number | null;
	/** Expected seconds between runs; `null` for streams, which run continuously. */
	every_secs: number | null;
}

export interface Status {
	claude_enabled: boolean;
	telegram_enabled: boolean;
	whale_alert_enabled: boolean;
	collectors: Collector[];
}

export interface PriceTick {
	asset: Asset;
	ts: number;
	price: number;
}

export const STREAM_URL = '/api/stream';

export class ApiError extends Error {
	constructor(
		message: string,
		readonly status: number | null
	) {
		super(message);
		this.name = 'ApiError';
	}
}

type Params = Record<string, string | number | undefined>;

async function get<T>(path: string, params: Params = {}): Promise<T> {
	const qs = new URLSearchParams();
	for (const [k, v] of Object.entries(params)) {
		if (v !== undefined) qs.set(k, String(v));
	}
	const query = qs.toString();
	const url = `/api${path}${query ? `?${query}` : ''}`;

	let res: Response;
	try {
		res = await fetch(url, { headers: { accept: 'application/json' } });
	} catch {
		throw new ApiError('서버 연결 안 됨', null);
	}
	if (!res.ok) {
		let message = `서버 오류 (HTTP ${res.status})`;
		try {
			const body: unknown = await res.json();
			if (body && typeof body === 'object' && 'error' in body && typeof body.error === 'string') {
				message = `${body.error} (HTTP ${res.status})`;
			}
		} catch {
			// non-JSON error body: keep the generic message
		}
		throw new ApiError(message, res.status);
	}
	try {
		return (await res.json()) as T;
	} catch {
		throw new ApiError('응답 형식 오류', res.status);
	}
}

export const api = {
	overview: (asset: Asset) => get<Overview>('/overview', { asset }),
	series: (metric: string, hours: number, mode: SeriesMode = 'raw') =>
		get<Series>('/series', { metric, hours, mode }),
	news: (limit = 50, beforeId?: number) =>
		get<NewsItem[]>('/news', { limit, before_id: beforeId }),
	events: (days = 14) => get<EventItem[]>('/events', { days }),
	alerts: (limit = 50) => get<Alert[]>('/alerts', { limit }),
	judgments: (asset: Asset, limit = 48, source?: JudgmentSource) =>
		get<Judgment[]>('/judgments', { asset, source, limit }),
	ledger: (asset: Asset) => get<Ledger>('/ledger', { asset }),
	brief: () => get<Brief>('/brief'),
	status: () => get<Status>('/status')
};

export function errorMessage(e: unknown): string {
	return e instanceof Error ? e.message : '알 수 없는 오류';
}
