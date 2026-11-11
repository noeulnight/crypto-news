<script lang="ts">
	import { api, type Metric, type Series, type SeriesMode } from '../api';
	import { fmtNum, fmtValue } from '../format';
	import { Resource, loadOn } from '../resource.svelte';
	import LineChart from './LineChart.svelte';
	import Loader from './Loader.svelte';

	let { metric }: { metric: Metric } = $props();

	const RANGES = [
		{ hours: 24, label: '24시간' },
		{ hours: 168, label: '7일' },
		{ hours: 720, label: '30일' }
	];
	const MODES: { mode: SeriesMode; label: string }[] = [
		{ mode: 'raw', label: '값' },
		{ mode: 'z', label: 'z-score' }
	];

	let hours = $state(168);
	let mode = $state<SeriesMode>('raw');

	const series = new Resource<Series>(() => api.series(metric.key, hours, mode));
	loadOn(
		() => [metric.key, hours, mode],
		() => void series.load(true)
	);

	const format = $derived(
		mode === 'z'
			? (v: number) => fmtNum(v, 2)
			: (v: number) => fmtValue(v, metric.unit, metric.decimals)
	);
</script>

<div class="detail">
	<div class="controls">
		<div class="switch" role="group" aria-label="기간">
			{#each RANGES as r (r.hours)}
				<button type="button" aria-pressed={hours === r.hours} onclick={() => (hours = r.hours)}>
					{r.label}
				</button>
			{/each}
		</div>
		<div class="switch" role="group" aria-label="표시 방식">
			{#each MODES as m (m.mode)}
				<button type="button" aria-pressed={mode === m.mode} onclick={() => (mode = m.mode)}>
					{m.label}
				</button>
			{/each}
		</div>
	</div>
	<Loader res={series}>
		{#snippet children(s)}
			<LineChart points={s.points} {format} height={280} />
		{/snippet}
	</Loader>
</div>

<style>
	.detail {
		padding: 14px 0 22px;
		border-bottom: 1px solid var(--line);
	}
	.controls {
		display: flex;
		justify-content: space-between;
		gap: 10px 28px;
		flex-wrap: wrap;
		margin-bottom: 10px;
		font-size: 14px;
	}
</style>
