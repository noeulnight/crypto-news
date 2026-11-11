<script lang="ts">
	import { onMount } from 'svelte';
	import { api, type Brief, type EventItem, type JudgmentSource } from '#lib/api.ts';
	import AlertList from '#lib/components/AlertList.svelte';
	import Judgment from '#lib/components/Judgment.svelte';
	import Loader from '#lib/components/Loader.svelte';
	import ScoreBar from '#lib/components/ScoreBar.svelte';
	import Sparkline from '#lib/components/Sparkline.svelte';
	import {
		CATEGORY_LABEL,
		SIGNAL_LABEL,
		fmtChange,
		fmtDateTime,
		fmtSigned,
		fmtStamp,
		fmtValue,
		isNum,
		label,
		signClass,
		signalClass
	} from '#lib/format.ts';
	import { Resource } from '#lib/resource.svelte.ts';
	import { app } from '#lib/state.svelte.ts';

	const SOURCES: JudgmentSource[] = ['claude', 'rules'];

	const events = new Resource<EventItem[]>(() => api.events());
	const brief = new Resource<Brief>(() => api.brief());

	onMount(() => {
		void events.load();
		void brief.load();
	});

	// Claude's judgment leads when there is one; the rule-based one is a tap away.
	let picked = $state<JudgmentSource | null>(null);
	const available = $derived(SOURCES.filter((s) => app.current?.judgments[s]));
	const shown = $derived(
		app.current?.judgments[picked && available.includes(picked) ? picked : (available[0] ?? 'rules')] ?? null
	);
	const metricLabels = $derived(new Map((app.current?.metrics ?? []).map((m) => [m.key, m.label])));
	const upcoming = $derived((events.data ?? []).filter((e) => e.ts > app.now).slice(0, 5));

	/** The more recent of the morning / weekly briefs. */
	function latestBrief(b: Brief) {
		const entries = [
			{ kind: '아침 브리핑', entry: b.morning },
			{ kind: '주간 요약', entry: b.weekly }
		].flatMap((x) => (x.entry ? [{ kind: x.kind, entry: x.entry }] : []));
		entries.sort((a, b) => b.entry.ts - a.entry.ts);
		return entries[0] ?? null;
	}
</script>

<svelte:head><title>국면 | Crypto News</title></svelte:head>

<div class="board">
	<div class="main">
		<Loader res={app.overview}>
			{#snippet children(ov)}
				{#if shown}
					<Judgment judgment={shown} {metricLabels} sources={available} onpick={(s) => (picked = s)} />
				{:else}
					<h1>판정 대기 중</h1>
					<p class="sub wait">가격이 들어오면 첫 판정이 나옵니다.</p>
				{/if}
				{#if !ov.claude_enabled}
					<p class="muted keyless">Claude 키가 없어 룰 판정만 나옵니다.</p>
				{/if}

				<section class="section">
					<div class="cat-head">
						<h2>종합 점수</h2>
						<span class="muted range">-100 약세, +100 강세</span>
					</div>
					<ScoreBar score={ov.scores.total} large />
				</section>

				{#each app.groups as g (g.category)}
					<section class="section">
						<div class="cat-head">
							<h2>{label(CATEGORY_LABEL, g.category)}</h2>
							<ScoreBar score={g.score} />
						</div>
						<div class="table-wrap">
							<table>
								<thead>
									<tr>
										<th>지표</th>
										<th class="spark" aria-hidden="true"></th>
										<th>현재</th>
										<th>24시간</th>
										<th>z</th>
									</tr>
								</thead>
								<tbody>
									{#each g.metrics as m (m.key)}
										<tr>
											<td class="name">
												{m.label}
												{#if m.signal !== 'neutral'}
													<span class="lean {signalClass(m.signal)}">{label(SIGNAL_LABEL, m.signal)}</span>
												{/if}
											</td>
											<td class="spark"><Sparkline points={m.spark} width={72} height={22} /></td>
											<td class="val">{fmtValue(m.value, m.unit, m.decimals)}</td>
											<td class={signClass(m.chg_24h)}>{fmtChange(m.chg_24h, m.unit, m.decimals)}</td>
											<td class:hot={isNum(m.z) && Math.abs(m.z) >= 2}>{fmtSigned(m.z)}</td>
										</tr>
									{/each}
								</tbody>
							</table>
						</div>
					</section>
				{/each}
			{/snippet}
		</Loader>
	</div>

	<aside>
		<section>
			<h2>알림</h2>
			<Loader res={app.alerts} isEmpty={(list) => list.length === 0}>
				{#snippet children(list)}
					<AlertList alerts={list.slice(0, 8)} />
				{/snippet}
			</Loader>
		</section>

		<section>
			<div class="side-head">
				<h2>일정</h2>
				<a class="link" href="/calendar">전체</a>
			</div>
			<Loader res={events} isEmpty={() => upcoming.length === 0}>
				{#snippet children()}
					<ul class="feed">
						{#each upcoming as e (e.id)}
							<li>
								<span class="chip" class:hot={e.impact === 'high'} title="{fmtDateTime(e.ts)} KST">
									{fmtStamp(e.ts, app.now)}
								</span>
								<div>
									<div class="etitle">{e.title}</div>
									{#if e.forecast || e.previous}
										<div class="sub figures">
											{#if e.forecast}예상 {e.forecast}{/if}
											{#if e.previous}이전 {e.previous}{/if}
										</div>
									{/if}
								</div>
							</li>
						{/each}
					</ul>
				{/snippet}
			</Loader>
		</section>

		<section>
			<h2>브리핑</h2>
			<Loader res={brief} isEmpty={(b) => latestBrief(b) === null}>
				{#snippet children(b)}
					{@const latest = latestBrief(b)}
					{#if latest}
						<p class="muted bmeta">{latest.kind}, {fmtDateTime(latest.entry.ts)}</p>
						<p class="brief">{latest.entry.body}</p>
					{/if}
				{/snippet}
			</Loader>
		</section>
	</aside>
</div>

<style>
	.board {
		display: grid;
		grid-template-columns: minmax(0, 1fr) 320px;
		gap: 64px;
		align-items: start;
	}
	.main {
		min-width: 0;
	}
	.wait {
		margin-top: 10px;
	}
	.keyless {
		margin-top: 12px;
		font-size: 13px;
	}
	.cat-head {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 8px 16px;
		flex-wrap: wrap;
		margin-bottom: 14px;
	}
	.range {
		font-size: 13px;
	}
	.name {
		white-space: normal;
		font-weight: 600;
	}
	.lean {
		margin-left: 6px;
		font-size: 13px;
	}
	.val {
		font-weight: 600;
	}
	.spark {
		width: 92px;
	}
	.hot {
		font-weight: 700;
	}

	aside {
		display: grid;
		gap: 44px;
		min-width: 0;
	}
	aside h2 {
		font-size: 18px;
		margin-bottom: 4px;
	}
	.side-head {
		display: flex;
		align-items: baseline;
		justify-content: space-between;
	}
	aside .feed > li {
		padding: 10px 0;
	}
	.etitle {
		font-weight: 600;
	}
	.figures {
		font-size: 14px;
	}
	.bmeta {
		font-size: 13px;
		margin-bottom: 8px;
	}
	.brief {
		font-size: 14px;
		white-space: pre-wrap;
	}

	@media (max-width: 980px) {
		.board {
			grid-template-columns: minmax(0, 1fr);
			gap: 56px;
		}
	}
	@media (max-width: 560px) {
		.spark {
			display: none;
		}
	}
</style>
