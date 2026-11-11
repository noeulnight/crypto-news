<script lang="ts">
	import type { Alert } from '../api';
	import { fmtDateTime, fmtStamp } from '../format';
	import { app } from '../state.svelte';

	let { alerts }: { alerts: Alert[] } = $props();

	// Briefings arrive here too; long bodies stay folded so the feed can be scanned.
	let expanded: Record<number, boolean> = $state({});
	const FOLD_LINES = 3;
	const isLong = (body: string) => body.length > 160 || body.split('\n').length > FOLD_LINES;
	/** Folded text skips blank lines so the preview is three lines of content. */
	const folded = (body: string) =>
		body
			.split('\n')
			.filter((line) => line.trim() !== '')
			.slice(0, FOLD_LINES)
			.join('\n');
</script>

<ul class="feed">
	{#each alerts as a (a.id)}
		<li>
			<span class="chip" class:hot={a.severity !== 'info'} title="{fmtDateTime(a.ts)} KST">
				{fmtStamp(a.ts, app.now)}
			</span>
			<div>
				<div class="title">{a.title}</div>
				{#if a.body}
					{@const long = isLong(a.body)}
					{@const fold = long && !expanded[a.id]}
					<p class="body sub" class:clamped={fold}>{fold ? folded(a.body) : a.body}</p>
					{#if long}
						<button type="button" class="link more" onclick={() => (expanded[a.id] = !expanded[a.id])}>
							{expanded[a.id] ? '접기' : '펼치기'}
						</button>
					{/if}
				{/if}
			</div>
		</li>
	{/each}
</ul>

<style>
	.feed > li {
		padding: 10px 0;
	}
	.title {
		font-weight: 600;
	}
	.body {
		font-size: 14px;
		white-space: pre-wrap;
	}
	.body.clamped {
		display: -webkit-box;
		-webkit-box-orient: vertical;
		-webkit-line-clamp: 3;
		line-clamp: 3;
		overflow: hidden;
	}
	.more {
		font-size: 13px;
	}
</style>
