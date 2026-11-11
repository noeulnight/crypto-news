<script lang="ts">
	import type { Point } from '../api';

	let {
		points,
		width = 96,
		height = 24
	}: { points: Point[]; width?: number; height?: number } = $props();

	const path = $derived.by(() => {
		const vals = points.map((p) => p[1]).filter((v) => typeof v === 'number' && Number.isFinite(v));
		if (vals.length < 2) return '';
		let min = vals[0];
		let max = vals[0];
		for (const v of vals) {
			if (v < min) min = v;
			if (v > max) max = v;
		}
		const span = max - min;
		const stepX = width / (vals.length - 1);
		// 1px inset so the stroke is not clipped; a flat series is drawn mid-height
		return vals
			.map((v, i) => {
				const y = span === 0 ? height / 2 : height - 1 - ((v - min) / span) * (height - 2);
				return `${i === 0 ? 'M' : 'L'}${(i * stepX).toFixed(1)} ${y.toFixed(1)}`;
			})
			.join('');
	});
</script>

<svg {width} {height} viewBox="0 0 {width} {height}" aria-hidden="true">
	{#if path}
		<path d={path} />
	{/if}
</svg>

<style>
	svg {
		display: block;
		flex: none;
	}
	path {
		fill: none;
		stroke: var(--ink-3);
		stroke-width: 1.25;
		stroke-linejoin: round;
		stroke-linecap: round;
	}
</style>
