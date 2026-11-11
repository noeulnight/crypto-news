import type { Bias, Confidence, JudgmentSource, Regime, Signal, Unit } from './api';

/** Placeholder for a value that is not there yet. */
export const NONE = '-';

export function isNum(v: number | null | undefined): v is number {
	return typeof v === 'number' && Number.isFinite(v);
}

function clampDecimals(d: number): number {
	return Number.isFinite(d) ? Math.min(8, Math.max(0, Math.trunc(d))) : 2;
}

/** Fixed decimals with thousands separators. */
export function fmtNum(v: number | null | undefined, decimals = 2): string {
	if (!isNum(v)) return NONE;
	const d = clampDecimals(decimals);
	return v.toLocaleString('en-US', { minimumFractionDigits: d, maximumFractionDigits: d });
}

const COMPACT_STEPS: readonly [number, string][] = [
	[1e12, 'T'],
	[1e9, 'B'],
	[1e6, 'M'],
	[1e3, 'K']
];

/** Unsigned compact magnitude: 1.2B / 350M / 12.5K; below 1,000 uses `decimals`. */
function compactAbs(abs: number, decimals: number): string {
	for (const [scale, suffix] of COMPACT_STEPS) {
		if (abs >= scale) {
			const n = abs / scale;
			const digits = n >= 100 ? 0 : n >= 10 ? 1 : 2;
			return `${parseFloat(n.toFixed(digits))}${suffix}`;
		}
	}
	return fmtNum(abs, decimals);
}

function body(abs: number, unit: Unit, decimals: number, isChange: boolean): string {
	switch (unit) {
		case '$':
			return `$${compactAbs(abs, decimals)}`;
		case '%':
			return `${fmtNum(abs, decimals)}${isChange ? '%p' : '%'}`;
		case 'bp':
			return `${fmtNum(abs, decimals)}bp`;
		default:
			// 'pt', 'coin' and plain numbers: thousands separators, no suffix
			return fmtNum(abs, decimals);
	}
}

function sign(v: number, forcePlus: boolean): string {
	if (v < 0) return '-';
	return forcePlus && v > 0 ? '+' : '';
}

/** Metric value by unit, e.g. `$1.2B`, `-$350M`, `0.0097%`, `12.5bp`, `1,234.5`. */
export function fmtValue(v: number | null | undefined, unit: Unit, decimals: number): string {
	if (!isNum(v)) return NONE;
	return sign(v, false) + body(Math.abs(v), unit, decimals, false);
}

/** Signed absolute change in the metric's unit, e.g. `+$1.2B`, `-0.0020%p`. */
export function fmtChange(v: number | null | undefined, unit: Unit, decimals: number): string {
	if (!isNum(v)) return NONE;
	return sign(v, true) + body(Math.abs(v), unit, decimals, true);
}

/** Signed percentage, e.g. `+1.47%`. */
export function fmtPct(v: number | null | undefined, decimals = 2): string {
	if (!isNum(v)) return NONE;
	return `${sign(v, true)}${fmtNum(Math.abs(v), decimals)}%`;
}

/** Full (non-compact) USD price, e.g. `$73,846.01`. */
export function fmtPrice(v: number | null | undefined): string {
	if (!isNum(v)) return NONE;
	return `$${fmtNum(v, Math.abs(v) >= 1000 ? 2 : Math.abs(v) >= 1 ? 3 : 5)}`;
}

/** Signed number, e.g. z-score `+1.3` or score `-12`. */
export function fmtSigned(v: number | null | undefined, decimals = 1): string {
	if (!isNum(v)) return NONE;
	return `${sign(v, true)}${fmtNum(Math.abs(v), decimals)}`;
}

/** CSS class for the Korean-market colour convention: rising = `up` (red), falling = `down` (blue). */
export function signClass(v: number | null | undefined): 'up' | 'down' | 'flat' {
	if (!isNum(v) || v === 0) return 'flat';
	return v > 0 ? 'up' : 'down';
}

export function signalClass(s: Signal): 'up' | 'down' | 'flat' {
	return s === 'bullish' ? 'up' : s === 'bearish' ? 'down' : 'flat';
}

// ---- time (KST, UTC+9) ----

export const KST_OFFSET = 9 * 3600;
const WEEKDAYS = ['일', '월', '화', '수', '목', '금', '토'];

function kst(ts: number): Date {
	// Shift then read with getUTC*: independent of the browser's timezone.
	return new Date((ts + KST_OFFSET) * 1000);
}

function p2(n: number): string {
	return String(n).padStart(2, '0');
}

/** `15:30` */
export function fmtTime(ts: number | null | undefined): string {
	if (!isNum(ts)) return NONE;
	const d = kst(ts);
	return `${p2(d.getUTCHours())}:${p2(d.getUTCMinutes())}`;
}

/** `11.11 15:30` */
export function fmtDateTime(ts: number | null | undefined): string {
	if (!isNum(ts)) return NONE;
	const d = kst(ts);
	return `${p2(d.getUTCMonth() + 1)}.${p2(d.getUTCDate())} ${fmtTime(ts)}`;
}

/** `11월 11일 (수)` */
export function fmtDate(ts: number | null | undefined): string {
	if (!isNum(ts)) return NONE;
	const d = kst(ts);
	return `${d.getUTCMonth() + 1}월 ${d.getUTCDate()}일 (${WEEKDAYS[d.getUTCDay()]})`;
}

/** KST calendar-day key, `2026-11-11`. */
export function kstDateKey(ts: number): string {
	const d = kst(ts);
	return `${d.getUTCFullYear()}-${p2(d.getUTCMonth() + 1)}-${p2(d.getUTCDate())}`;
}

/** Feed timestamp: the time for today, the date for any other day. */
export function fmtStamp(ts: number, now: number): string {
	if (kstDateKey(ts) === kstDateKey(now)) return fmtTime(ts);
	const d = kst(ts);
	return `${p2(d.getUTCMonth() + 1)}.${p2(d.getUTCDate())}`;
}

/** `3분 전` / `2시간 뒤` */
export function relTime(ts: number | null | undefined, now: number): string {
	if (!isNum(ts)) return NONE;
	const diff = now - ts;
	const abs = Math.abs(diff);
	if (abs < 60) return diff >= 0 ? '방금' : '곧';
	const suffix = diff >= 0 ? '전' : '뒤';
	if (abs < 3600) return `${Math.floor(abs / 60)}분 ${suffix}`;
	if (abs < 86400) return `${Math.floor(abs / 3600)}시간 ${suffix}`;
	return `${Math.floor(abs / 86400)}일 ${suffix}`;
}

/** Countdown `2일 03:14:05` for a non-negative number of seconds. */
export function fmtCountdown(seconds: number): string {
	const s = Math.max(0, Math.floor(seconds));
	const days = Math.floor(s / 86400);
	const hms = `${p2(Math.floor((s % 86400) / 3600))}:${p2(Math.floor((s % 3600) / 60))}:${p2(s % 60)}`;
	return days > 0 ? `${days}일 ${hms}` : hms;
}

// ---- enum → Korean labels ----

export const REGIME_LABEL: Record<Regime, string> = {
	spot_led_rally: '현물 주도 상승',
	leverage_led_rally: '레버리지 주도 상승',
	short_squeeze: '숏 스퀴즈',
	spot_led_selloff: '현물 주도 하락',
	long_liquidation_cascade: '롱 청산 연쇄',
	macro_risk_off: '거시발 위험 회피',
	volatility_compression: '변동성 수축',
	event_wait: '이벤트 대기',
	range_chop: '방향 없음'
};

export const BIAS_LABEL: Record<Bias, string> = {
	strong_down: '강한 하락',
	down: '하락',
	neutral: '중립',
	up: '상승',
	strong_up: '강한 상승'
};

export const CONFIDENCE_LABEL: Record<Confidence, string> = {
	low: '낮음',
	mid: '보통',
	high: '높음'
};

export const CATEGORY_LABEL: Record<string, string> = {
	flow: '자금 흐름',
	derivs: '파생',
	macro: '거시',
	onchain: '온체인'
};

export const SIGNAL_LABEL: Record<Signal, string> = {
	bullish: '강세',
	bearish: '약세',
	neutral: '중립'
};

export const SOURCE_LABEL: Record<JudgmentSource, string> = {
	claude: 'Claude',
	rules: '룰'
};

export const NEWS_ASSET_LABEL: Record<string, string> = {
	btc: 'BTC',
	eth: 'ETH',
	alt: '알트',
	macro: '거시',
	regulation: '규제',
	other: '기타'
};

/** Label lookup that never renders `undefined` for a value outside the known enum. */
export function label(map: Record<string, string>, key: string | null | undefined): string {
	if (key === null || key === undefined || key === '') return NONE;
	return map[key] ?? key;
}

export function biasClass(b: Bias): 'up' | 'down' | 'flat' {
	return b === 'up' || b === 'strong_up' ? 'up' : b === 'down' || b === 'strong_down' ? 'down' : 'flat';
}

/** Resolved value of a CSS custom property (for canvas drawing, which cannot use `var()`). */
export function cssVar(name: string): string {
	return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}
