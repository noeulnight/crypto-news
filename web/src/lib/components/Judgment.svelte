<script lang="ts">
	import type { Judgment, JudgmentSource } from '../api';
	import {
		BIAS_LABEL,
		CONFIDENCE_LABEL,
		REGIME_LABEL,
		SIGNAL_LABEL,
		SOURCE_LABEL,
		biasClass,
		fmtDateTime,
		fmtPrice,
		label,
		signalClass
	} from '../format';

	let {
		judgment: j,
		metricLabels,
		sources,
		onpick
	}: {
		judgment: Judgment;
		/** metric key → display label, from the overview */
		metricLabels: Map<string, string>;
		/** judgment sources that currently have a result */
		sources: JudgmentSource[];
		onpick: (source: JudgmentSource) => void;
	} = $props();
</script>

<article>
	<div class="head">
		<h1>{label(REGIME_LABEL, j.regime)}</h1>
		{#if sources.length > 1}
			<div class="switch" role="group" aria-label="판정 출처">
				{#each sources as s (s)}
					<button type="button" aria-pressed={s === j.source} onclick={() => onpick(s)}>
						{label(SOURCE_LABEL, s)}
					</button>
				{/each}
			</div>
		{/if}
	</div>

	<dl class="calls">
		<div>
			<dt>4시간</dt>
			<dd class={biasClass(j.bias_4h)}>{label(BIAS_LABEL, j.bias_4h)}</dd>
		</div>
		<div>
			<dt>24시간</dt>
			<dd class={biasClass(j.bias_24h)}>{label(BIAS_LABEL, j.bias_24h)}</dd>
		</div>
		<div>
			<dt>신뢰도</dt>
			<dd>{label(CONFIDENCE_LABEL, j.confidence)}</dd>
		</div>
	</dl>

	{#if j.summary}
		<p class="summary">{j.summary}</p>
	{/if}

	{#if j.drivers.length > 0}
		<ul class="drivers" aria-label="근거">
			{#each j.drivers as d, i (i)}
				<li>
					<span class="name">{metricLabels.get(d.metric) ?? d.metric}</span>
					<span class="sub">{d.note}</span>
					<span class="dir {signalClass(d.direction)}">{label(SIGNAL_LABEL, d.direction)}</span>
				</li>
			{/each}
		</ul>
	{/if}

	<dl class="notes">
		{#if j.conflicts}
			<div>
				<dt>반대 신호</dt>
				<dd>{j.conflicts}</dd>
			</div>
		{/if}
		{#if j.invalidation}
			<div>
				<dt>무효 조건</dt>
				<dd>{j.invalidation}</dd>
			</div>
		{/if}
	</dl>

	<p class="stamp muted">
		{fmtDateTime(j.ts)} {label(SOURCE_LABEL, j.source)} 판정, 당시 {fmtPrice(j.price)}
	</p>
</article>

<style>
	.head {
		display: flex;
		align-items: baseline;
		justify-content: space-between;
		gap: 8px 20px;
		flex-wrap: wrap;
	}
	.calls {
		display: flex;
		flex-wrap: wrap;
		gap: 10px 36px;
		margin-top: 18px;
	}
	.calls dt {
		color: var(--ink-3);
		font-size: 13px;
	}
	.calls dd {
		font-size: 19px;
		font-weight: 700;
		letter-spacing: -0.02em;
	}
	.summary {
		max-width: 62ch;
		margin-top: 20px;
		font-size: 17px;
		line-height: 1.65;
	}
	.drivers {
		list-style: none;
		margin: 24px 0 0;
		padding: 0;
	}
	.drivers li {
		display: grid;
		grid-template-columns: minmax(0, 14em) minmax(0, 1fr) auto;
		gap: 2px 16px;
		align-items: baseline;
		padding: 10px 0;
		border-bottom: 1px solid var(--line);
	}
	.drivers li:first-child {
		border-top: 1px solid var(--line);
	}
	.name {
		font-weight: 600;
	}
	.dir {
		font-size: 14px;
		font-weight: 600;
	}
	.notes {
		margin-top: 18px;
		display: grid;
		gap: 6px;
	}
	.notes div {
		display: grid;
		grid-template-columns: 5em minmax(0, 1fr);
		gap: 12px;
	}
	.notes dt {
		color: var(--ink-3);
		font-size: 14px;
	}
	.stamp {
		margin-top: 16px;
		font-size: 13px;
	}

	@media (max-width: 560px) {
		.drivers li {
			grid-template-columns: minmax(0, 1fr) auto;
		}
		.drivers .sub {
			grid-column: 1 / -1;
			grid-row: 2;
		}
	}
</style>
