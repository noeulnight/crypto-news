<script lang="ts">
	import { fmtSigned, isNum, signClass } from '../format';

	let { score, large = false }: { score: number | null; large?: boolean } = $props();

	// -100..+100 mapped onto half the track on either side of the centre line
	const half = $derived(isNum(score) ? Math.min(100, Math.abs(score)) / 2 : 0);
	const cls = $derived(signClass(score));
</script>

<div class="score" class:large>
	<span class="value {cls}">{isNum(score) ? fmtSigned(score, 0) : '수집 중'}</span>
	<div class="track" role="img" aria-label="점수 {fmtSigned(score, 0)}, 범위 -100에서 +100">
		{#if isNum(score) && score !== 0}
			<div
				class="fill {cls}"
				style:width="{half}%"
				style:left={score > 0 ? '50%' : `${50 - half}%`}
			></div>
		{/if}
		<div class="centre"></div>
	</div>
</div>

<style>
	.score {
		display: flex;
		align-items: center;
		gap: 12px;
	}
	.value {
		min-width: 2.4em;
		text-align: right;
		font-size: 17px;
		font-weight: 700;
		white-space: nowrap;
	}
	.track {
		position: relative;
		width: 140px;
		height: 4px;
		border-radius: 2px;
		background: var(--fill-2);
	}
	.fill {
		position: absolute;
		top: 0;
		bottom: 0;
		border-radius: inherit;
	}
	.fill.up {
		background: var(--up);
	}
	.fill.down {
		background: var(--down);
	}
	.centre {
		position: absolute;
		left: 50%;
		top: -4px;
		bottom: -4px;
		width: 1px;
		background: var(--ink-3);
	}
	.large .value {
		font-size: 28px;
		letter-spacing: -0.03em;
	}
	.large .track {
		flex: 1;
		width: auto;
		height: 6px;
		border-radius: 3px;
	}
</style>
