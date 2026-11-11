<script lang="ts">
	import { onMount } from 'svelte';
	import { api, type EventItem } from '#lib/api.ts';
	import Loader from '#lib/components/Loader.svelte';
	import { fmtCountdown, fmtDate, fmtDateTime, fmtTime, kstDateKey } from '#lib/format.ts';
	import { Resource } from '#lib/resource.svelte.ts';
	import { app } from '#lib/state.svelte.ts';

	const events = new Resource<EventItem[]>(() => api.events(30));
	onMount(() => void events.load());

	const days = $derived.by(() => {
		const byDay = new Map<string, EventItem[]>();
		for (const e of events.data ?? []) {
			const key = kstDateKey(e.ts);
			const list = byDay.get(key);
			if (list) list.push(e);
			else byDay.set(key, [e]);
		}
		return [...byDay].map(([key, list]) => ({ key, list }));
	});
	const next = $derived((events.data ?? []).find((e) => e.ts > app.now) ?? null);
	const todayKey = $derived(kstDateKey(app.now));
</script>

<svelte:head><title>일정 | Crypto News</title></svelte:head>

<div class="read">
	<div class="pagehead">
		<h1>일정</h1>
	</div>

	<Loader res={events} isEmpty={(list) => list.length === 0}>
		{#snippet children()}
			{#if next}
				<div class="next">
					<div class="countdown">{fmtCountdown(next.ts - app.now)}</div>
					<p class="ntitle">{next.title}</p>
					<p class="muted">{fmtDateTime(next.ts)} 발표</p>
				</div>
			{/if}

			{#each days as d (d.key)}
				<div class="daybar">
					{fmtDate(d.list[0].ts)}{#if d.key === todayKey}<span class="today">오늘</span>{/if}
				</div>
				<ul class="feed">
					{#each d.list as e (e.id)}
						{@const past = e.ts <= app.now}
						<li class:past>
							<span class="chip" class:hot={!past && e.impact === 'high'}>{fmtTime(e.ts)}</span>
							<div class="row">
								<span class="etitle">{e.title}</span>
								<span class="figures sub">
									{#if e.forecast}<span>예상 {e.forecast}</span>{/if}
									{#if e.previous}<span>이전 {e.previous}</span>{/if}
									{#if past}<span>지남</span>{/if}
								</span>
							</div>
						</li>
					{/each}
				</ul>
			{/each}
		{/snippet}
	</Loader>
</div>

<style>
	.countdown {
		font-size: 44px;
		line-height: 1.15;
		font-weight: 700;
		letter-spacing: -0.04em;
	}
	.ntitle {
		margin-top: 6px;
		font-size: 19px;
		font-weight: 600;
		letter-spacing: -0.02em;
	}
	.today {
		margin-left: 8px;
		color: var(--hot);
	}
	.feed > li {
		padding: 12px 0;
	}
	.row {
		display: flex;
		align-items: baseline;
		justify-content: space-between;
		gap: 2px 20px;
		flex-wrap: wrap;
	}
	.etitle {
		font-weight: 600;
	}
	.figures {
		display: inline-flex;
		gap: 14px;
		font-size: 14px;
		white-space: nowrap;
	}
	.past .etitle,
	.past .figures {
		color: var(--ink-3);
		font-weight: 400;
	}
</style>
