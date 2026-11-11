import {
	api,
	ASSETS,
	STREAM_URL,
	type Alert,
	type Asset,
	type Judgment,
	type Metric,
	type Overview,
	type PriceTick,
	type Status
} from './api';
import { Resource } from './resource.svelte';

const ASSET_KEY = 'crypto-news.asset';
const SEEN_ALERT_KEY = 'crypto-news.seenAlertId';
const POLL_MS = 60_000;
const MAX_ALERTS = 100;

function readStored(key: string): string | null {
	try {
		return localStorage.getItem(key);
	} catch {
		return null;
	}
}

function writeStored(key: string, value: string): void {
	try {
		localStorage.setItem(key, value);
	} catch {
		// storage unavailable (private mode): the choice just isn't persisted
	}
}

function storedAsset(): Asset {
	const v = readStored(ASSET_KEY);
	return ASSETS.find((a) => a === v) ?? 'btc';
}

function parse<T>(e: MessageEvent<string>): T | null {
	try {
		return JSON.parse(e.data) as T;
	} catch {
		return null;
	}
}

export interface MetricGroup {
	category: string;
	score: number | null;
	metrics: Metric[];
}

export type NotifyState = NotificationPermission | 'unsupported';

class AppState {
	asset = $state<Asset>(storedAsset());
	/** Wall clock in unix seconds, ticking every second while the app is mounted. */
	now = $state(Math.floor(Date.now() / 1000));
	streamConnected = $state(false);
	notify = $state<NotifyState>(
		typeof Notification === 'undefined' ? 'unsupported' : Notification.permission
	);

	readonly overview = new Resource<Overview>(() => api.overview(this.asset));
	readonly alerts = new Resource<Alert[]>(() => api.alerts(50));
	readonly status = new Resource<Status>(() => api.status());

	#live = $state<Partial<Record<Asset, PriceTick>>>({});
	#seenAlertId = $state(Number(readStored(SEEN_ALERT_KEY)) || 0);

	/** Overview for the selected asset only (never the previous asset's while switching). */
	readonly current = $derived.by(() => {
		const ov = this.overview.data;
		return ov && ov.asset === this.asset ? ov : null;
	});

	/** Latest price: the SSE tick when it is newer than the polled overview. */
	readonly price = $derived.by(() => {
		const live = this.#live[this.asset];
		const ov = this.current;
		if (live && (!ov || live.ts >= ov.ts)) return live.price;
		return ov ? ov.price : null;
	});

	/** Category scores with their metrics; categories come from the overview payload. */
	readonly groups = $derived.by(() => {
		const ov = this.current;
		const groups = new Map<string, MetricGroup>();
		if (!ov) return [];
		for (const [category, score] of Object.entries(ov.scores)) {
			if (category !== 'total') groups.set(category, { category, score, metrics: [] });
		}
		for (const m of ov.metrics) {
			let g = groups.get(m.category);
			if (!g) {
				g = { category: m.category, score: null, metrics: [] };
				groups.set(m.category, g);
			}
			g.metrics.push(m);
		}
		return [...groups.values()];
	});

	readonly unread = $derived(
		(this.alerts.data ?? []).filter((a) => a.id > this.#seenAlertId).length
	);

	setAsset(asset: Asset): void {
		if (asset === this.asset) return;
		this.asset = asset;
		writeStored(ASSET_KEY, asset);
	}

	markAlertsSeen(): void {
		const maxId = (this.alerts.data ?? []).reduce((m, a) => Math.max(m, a.id), this.#seenAlertId);
		this.#seenAlertId = maxId;
		writeStored(SEEN_ALERT_KEY, String(maxId));
	}

	async enableNotifications(): Promise<void> {
		if (typeof Notification === 'undefined') return;
		this.notify = await Notification.requestPermission();
	}

	/** Starts the clock, polling and the single shared SSE connection. Returns the cleanup. */
	start(): () => void {
		const tick = setInterval(() => (this.now = Math.floor(Date.now() / 1000)), 1000);
		const poll = setInterval(() => {
			void this.overview.load();
			void this.status.load();
		}, POLL_MS);
		void this.alerts.load();
		void this.status.load();

		const es = new EventSource(STREAM_URL);
		es.onopen = () => (this.streamConnected = true);
		// EventSource reconnects on its own; this only reflects the state in the UI.
		es.onerror = () => (this.streamConnected = false);
		es.addEventListener('price', (e) => this.#onPrice(parse<PriceTick>(e)));
		es.addEventListener('alert', (e) => this.#onAlert(parse<Alert>(e)));
		es.addEventListener('judgment', (e) => this.#onJudgment(parse<Judgment>(e)));

		return () => {
			clearInterval(tick);
			clearInterval(poll);
			es.close();
			this.streamConnected = false;
		};
	}

	#onPrice(t: PriceTick | null): void {
		if (!t || typeof t.price !== 'number') return;
		this.#live[t.asset] = t;
	}

	#onAlert(a: Alert | null): void {
		if (!a) return;
		const list = this.alerts.data ?? [];
		if (list.some((x) => x.id === a.id)) return;
		this.alerts.data = [a, ...list].slice(0, MAX_ALERTS);

		if (this.notify === 'granted' && (a.severity === 'warn' || a.severity === 'strong')) {
			try {
				new Notification(a.title, { body: a.body, tag: `alert-${a.id}` });
			} catch {
				// some mobile browsers only allow notifications from a service worker
			}
		}
	}

	#onJudgment(j: Judgment | null): void {
		const ov = this.overview.data;
		if (!j || !ov || j.asset !== ov.asset) return;
		this.overview.data = { ...ov, judgments: { ...ov.judgments, [j.source]: j } };
	}
}

export const app = new AppState();
