<script lang="ts" module>
	export interface ChartMarker {
		/** unix seconds */
		ts: number;
		/** resolved colour string (canvas cannot use CSS variables) */
		color: string;
	}
</script>

<script lang="ts">
	import { onMount } from 'svelte';
	import {
		ColorType,
		LineSeries,
		createChart,
		createSeriesMarkers,
		type IChartApi,
		type ISeriesApi,
		type ISeriesMarkersPluginApi,
		type LineData,
		type SeriesMarker,
		type Time,
		type UTCTimestamp
	} from 'lightweight-charts';
	import type { Point } from '../api';
	import { KST_OFFSET, cssVar, fmtNum } from '../format';

	let {
		points,
		markers = [],
		height = 260,
		format = (v: number) => fmtNum(v, 2)
	}: {
		points: Point[];
		markers?: ChartMarker[];
		height?: number;
		/** price-axis / crosshair value formatter */
		format?: (v: number) => string;
	} = $props();

	let el: HTMLDivElement;
	let chart = $state.raw<IChartApi>();
	let series = $state.raw<ISeriesApi<'Line'>>();
	let markerApi: ISeriesMarkersPluginApi<Time> | undefined;

	// The library renders timestamps as UTC, so they are shifted to show KST wall-clock time.
	const data = $derived.by(() => {
		const out: LineData<UTCTimestamp>[] = [];
		let last = -Infinity;
		for (const [ts, value] of points) {
			// times must be strictly ascending
			if (typeof value !== 'number' || !Number.isFinite(value) || ts <= last) continue;
			out.push({ time: (ts + KST_OFFSET) as UTCTimestamp, value });
			last = ts;
		}
		return out;
	});

	/** Snap each marker to the nearest data point; markers outside the plotted range are dropped. */
	const seriesMarkers = $derived.by(() => {
		if (data.length === 0) return [];
		const first = data[0].time;
		const lastTime = data[data.length - 1].time;
		const out: SeriesMarker<Time>[] = [];
		for (const m of markers) {
			const t = m.ts + KST_OFFSET;
			if (t < first || t > lastTime) continue;
			let lo = 0;
			let hi = data.length - 1;
			while (lo < hi) {
				const mid = (lo + hi) >> 1;
				if (data[mid].time < t) lo = mid + 1;
				else hi = mid;
			}
			const snapped = lo > 0 && t - data[lo - 1].time < data[lo].time - t ? data[lo - 1] : data[lo];
			out.push({ time: snapped.time, position: 'aboveBar', shape: 'circle', color: m.color });
		}
		return out.sort((a, b) => (a.time as number) - (b.time as number));
	});

	onMount(() => {
		const c = createChart(el, {
			autoSize: true,
			layout: {
				background: { type: ColorType.Solid, color: 'transparent' },
				textColor: cssVar('--ink-3'),
				fontSize: 12,
				fontFamily: cssVar('--font')
			},
			grid: { vertLines: { visible: false }, horzLines: { color: cssVar('--line') } },
			rightPriceScale: { borderVisible: false },
			timeScale: { borderVisible: false, timeVisible: true, secondsVisible: false },
			localization: { locale: 'ko-KR' }
		});
		const s = c.addSeries(LineSeries, {
			color: cssVar('--ink'),
			lineWidth: 2,
			priceLineVisible: false
		});
		markerApi = createSeriesMarkers(s, []);
		chart = c;
		series = s;
		return () => {
			chart = undefined;
			series = undefined;
			markerApi = undefined;
			c.remove();
		};
	});

	$effect(() => {
		chart?.applyOptions({ localization: { priceFormatter: format } });
	});

	$effect(() => {
		if (!chart || !series) return;
		series.setData(data);
		markerApi?.setMarkers(seriesMarkers);
		chart.timeScale().fitContent();
	});
</script>

<div class="chart" style:height="{height}px">
	<div class="canvas" bind:this={el}></div>
	{#if data.length === 0}
		<div class="empty overlay">아직 없음</div>
	{/if}
</div>

<style>
	.chart {
		position: relative;
		width: 100%;
		min-width: 0;
	}
	.canvas {
		position: absolute;
		inset: 0;
	}
	.overlay {
		position: absolute;
		inset: 0;
		display: grid;
		place-items: center;
		pointer-events: none;
	}
</style>
