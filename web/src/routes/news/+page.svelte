<script lang="ts">
	import { onMount } from 'svelte';
	import { api, errorMessage, type Asset, type NewsItem, type Series } from '#lib/api.ts';
	import ErrorBox from '#lib/components/ErrorBox.svelte';
	import LineChart, { type ChartMarker } from '#lib/components/LineChart.svelte';
	import Loader from '#lib/components/Loader.svelte';
	import {
		NEWS_ASSET_LABEL,
		SIGNAL_LABEL,
		cssVar,
		fmtDate,
		fmtPct,
		fmtPrice,
		fmtTime,
		isNum,
		kstDateKey,
		label,
		signClass,
		signalClass
	} from '#lib/format.ts';
	import { Resource } from '#lib/resource.svelte.ts';

	// News reactions are measured against BTC, so the chart is BTC regardless of the selected asset.
	const CHART_ASSET: Asset = 'btc';
	const CHART_HOURS = 48;
	const PAGE_SIZE = 50;

	const price = new Resource<Series>(() => api.series(`${CHART_ASSET}.price`, CHART_HOURS));

	let items = $state.raw<NewsItem[]>([]);
	let loaded = $state(false);
	let loading = $state(false);
	let exhausted = $state(false);
	let error = $state<string | null>(null);
	let openIds = $state.raw<ReadonlySet<number>>(new Set());

	async function loadMore() {
		if (loading) return;
		loading = true;
		error = null;
		try {
			const page = await api.news(PAGE_SIZE, items.at(-1)?.id);
			items = [...items, ...page];
			exhausted = page.length < PAGE_SIZE;
			loaded = true;
		} catch (e) {
			error = errorMessage(e);
		} finally {
			loading = false;
		}
	}

	onMount(() => {
		void price.load();
		void loadMore();
	});

	function isMajor(n: NewsItem): boolean {
		return n.important || (n.tag?.importance ?? 0) >= 3;
	}

	const days = $derived.by(() => {
		const byDay = new Map<string, NewsItem[]>();
		for (const n of items) {
			const key = kstDateKey(n.ts);
			const list = byDay.get(key);
			if (list) list.push(n);
			else byDay.set(key, [n]);
		}
		return [...byDay].map(([key, list]) => ({ key, list }));
	});

	const markers = $derived.by((): ChartMarker[] => {
		const color = cssVar('--hot');
		return items.filter(isMajor).map((n) => ({ ts: n.ts, color }));
	});

	function toggle(id: number) {
		const next = new Set(openIds);
		if (!next.delete(id)) next.add(id);
		openIds = next;
	}

	/** Only http(s) links are rendered as anchors. */
	function safeLink(link: string | null): string | null {
		return link !== null && /^https?:\/\//i.test(link) ? link : null;
	}
</script>

<svelte:head><title>속보 | Crypto News</title></svelte:head>

<div class="read">
	<div class="pagehead">
		<h1>속보</h1>
	</div>

	<div class="chart-cap">
		<span class="cap-title">{CHART_ASSET.toUpperCase()} {CHART_HOURS}시간</span>
		<span class="muted">주황 점이 주요 속보</span>
	</div>
	<Loader res={price}>
		{#snippet children(s)}
			<LineChart points={s.points} {markers} format={fmtPrice} height={240} />
		{/snippet}
	</Loader>

	{#if loaded && items.length === 0}
		<div class="empty">아직 들어온 속보가 없습니다</div>
	{/if}

	{#each days as d (d.key)}
		<div class="daybar">{fmtDate(d.list[0].ts)}</div>
		<ul class="feed">
			{#each d.list as n (n.id)}
				{@const open = openIds.has(n.id)}
				{@const href = safeLink(n.link)}
				{@const major = isMajor(n)}
				<li>
					<span class="chip" class:hot={major}>{fmtTime(n.ts)}</span>
					<div>
						<button
							class="title"
							class:hot={major}
							type="button"
							aria-expanded={open}
							onclick={() => toggle(n.id)}
						>
							{n.title}
						</button>

						{#if n.content}
							<p class="content sub" class:clamped={!open}>{n.content}</p>
						{/if}
						{#if open && href}
							<a class="link" {href} target="_blank" rel="noopener noreferrer">원문 보기</a>
						{/if}

						<div class="meta muted">
							<span>강세 {n.bull}</span>
							<span>약세 {n.bear}</span>
							{#if n.tag}
								<span>
									{label(NEWS_ASSET_LABEL, n.tag.asset)}
									<span class={signalClass(n.tag.direction)}>{label(SIGNAL_LABEL, n.tag.direction)}</span>
								</span>
							{/if}
							{#if isNum(n.react_5m_pct)}
								<span>5분 뒤 <span class={signClass(n.react_5m_pct)}>{fmtPct(n.react_5m_pct)}</span></span>
							{/if}
							{#if isNum(n.react_1h_pct)}
								<span>1시간 뒤 <span class={signClass(n.react_1h_pct)}>{fmtPct(n.react_1h_pct)}</span></span>
							{/if}
						</div>
					</div>
				</li>
			{/each}
		</ul>
	{/each}

	{#if error !== null}
		<ErrorBox message={error} onretry={loadMore} />
	{:else if !loaded}
		<div class="empty">불러오는 중</div>
	{:else if !exhausted}
		<div class="more">
			<button class="btn" type="button" disabled={loading} onclick={loadMore}>
				{loading ? '불러오는 중' : '이전 속보 더 보기'}
			</button>
		</div>
	{/if}
</div>

<style>
	.chart-cap {
		display: flex;
		align-items: baseline;
		justify-content: space-between;
		gap: 12px;
		margin-bottom: 6px;
		font-size: 14px;
	}
	.cap-title {
		font-weight: 600;
	}
	.feed > li {
		padding: 18px 0;
	}
	.title {
		display: block;
		width: 100%;
		text-align: left;
		font-size: 17px;
		font-weight: 600;
		line-height: 1.45;
		letter-spacing: -0.02em;
	}
	.title:hover {
		text-decoration: underline;
		text-underline-offset: 3px;
	}
	.content {
		margin-top: 4px;
		white-space: pre-wrap;
	}
	.content.clamped {
		display: -webkit-box;
		-webkit-box-orient: vertical;
		-webkit-line-clamp: 3;
		line-clamp: 3;
		overflow: hidden;
	}
	.link {
		display: inline-block;
		margin-top: 6px;
	}
	.meta {
		display: flex;
		gap: 4px 16px;
		flex-wrap: wrap;
		margin-top: 8px;
		font-size: 13px;
	}
	.more {
		padding-top: 20px;
	}
</style>
