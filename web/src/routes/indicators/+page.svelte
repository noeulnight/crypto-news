<script lang="ts">
	import Loader from '#lib/components/Loader.svelte';
	import MetricChart from '#lib/components/MetricChart.svelte';
	import ScoreBar from '#lib/components/ScoreBar.svelte';
	import Sparkline from '#lib/components/Sparkline.svelte';
	import {
		CATEGORY_LABEL,
		SIGNAL_LABEL,
		fmtChange,
		fmtSigned,
		fmtValue,
		isNum,
		label,
		relTime,
		signClass,
		signalClass
	} from '#lib/format.ts';
	import { app } from '#lib/state.svelte.ts';

	let expanded = $state<string | null>(null);
</script>

<svelte:head><title>지표 | Crypto News</title></svelte:head>

<div class="pagehead">
	<h1>지표</h1>
	<p class="muted">누르면 차트가 열립니다. z는 최근 30일과 비교한 값입니다.</p>
</div>

<Loader res={app.overview} isEmpty={() => app.groups.every((g) => g.metrics.length === 0)}>
	{#snippet children()}
		{#each app.groups as g (g.category)}
			<section class="section">
				<div class="cat-head">
					<h2>{label(CATEGORY_LABEL, g.category)}</h2>
					<ScoreBar score={g.score} />
				</div>

				<div class="grid head" aria-hidden="true">
					<span>지표</span>
					<span></span>
					<span class="r">현재</span>
					<span class="r">1시간</span>
					<span class="r">24시간</span>
					<span class="r">z</span>
				</div>

				{#each g.metrics as m (m.key)}
					{@const open = expanded === m.key}
					<button
						class="grid rowbtn"
						type="button"
						aria-expanded={open}
						onclick={() => (expanded = open ? null : m.key)}
					>
						<span class="lbl">
							<span class="name">
								{m.label}
								{#if m.signal !== 'neutral'}
									<span class="lean {signalClass(m.signal)}">{label(SIGNAL_LABEL, m.signal)}</span>
								{/if}
							</span>
							<span class="note muted">
								{m.note}{#if isNum(m.ts)}<span class="age">{relTime(m.ts, app.now)}</span>{/if}
							</span>
						</span>
						<span class="spark"><Sparkline points={m.spark} /></span>
						<span class="val">{fmtValue(m.value, m.unit, m.decimals)}</span>
						<span class="c1 {signClass(m.chg_1h)}" data-label="1시간">
							{fmtChange(m.chg_1h, m.unit, m.decimals)}
						</span>
						<span class="c24 {signClass(m.chg_24h)}" data-label="24시간">
							{fmtChange(m.chg_24h, m.unit, m.decimals)}
						</span>
						<span class="z" class:hot={isNum(m.z) && Math.abs(m.z) >= 2} data-label="z">
							{fmtSigned(m.z)}
						</span>
					</button>
					{#if open}
						<MetricChart metric={m} />
					{/if}
				{/each}
			</section>
		{/each}
	{/snippet}
</Loader>

<style>
	.pagehead p {
		margin-top: 6px;
		font-size: 14px;
	}
	.cat-head {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 8px 16px;
		flex-wrap: wrap;
		margin-bottom: 14px;
	}
	.grid {
		display: grid;
		grid-template-columns: minmax(0, 1fr) 96px 112px 104px 104px 56px;
		grid-template-areas: 'lbl spark val c1 c24 z';
		align-items: center;
		gap: 4px 20px;
		width: 100%;
		padding: 12px 0;
		border-bottom: 1px solid var(--line);
		text-align: left;
		white-space: nowrap;
	}
	.head {
		padding: 0 0 8px;
		color: var(--ink-3);
		font-size: 13px;
	}
	.r {
		text-align: right;
	}
	.rowbtn:hover .name,
	.rowbtn[aria-expanded='true'] .name {
		text-decoration: underline;
		text-underline-offset: 3px;
	}
	.lbl {
		grid-area: lbl;
		min-width: 0;
		display: flex;
		flex-direction: column;
		white-space: normal;
	}
	.name {
		font-weight: 600;
	}
	.lean {
		margin-left: 6px;
		font-size: 13px;
	}
	.note {
		font-size: 13px;
	}
	.age {
		margin-left: 8px;
		white-space: nowrap;
	}
	.spark {
		grid-area: spark;
	}
	.val {
		grid-area: val;
		text-align: right;
		font-weight: 600;
	}
	.c1 {
		grid-area: c1;
		text-align: right;
	}
	.c24 {
		grid-area: c24;
		text-align: right;
	}
	.z {
		grid-area: z;
		text-align: right;
	}
	.z.hot {
		font-weight: 700;
	}

	@media (max-width: 820px) {
		.head {
			display: none;
		}
		.grid {
			grid-template-columns: minmax(0, 1fr) auto;
			grid-template-areas:
				'lbl val'
				'meta meta';
			gap: 6px 14px;
		}
		.spark {
			display: none;
		}
		.c1,
		.c24,
		.z {
			grid-area: auto;
			grid-row: 2;
			text-align: left;
			font-size: 13px;
		}
		.grid {
			grid-template-columns: repeat(3, auto) minmax(0, 1fr);
			grid-template-areas: 'lbl lbl lbl val';
		}
		.c1::before,
		.c24::before,
		.z::before {
			content: attr(data-label) ' ';
			color: var(--ink-3);
			font-weight: 400;
		}
	}
</style>
