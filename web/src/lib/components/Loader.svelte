<script lang="ts" generics="T">
	import type { Snippet } from 'svelte';
	import type { Resource } from '../resource.svelte';
	import ErrorBox from './ErrorBox.svelte';

	let {
		res,
		isEmpty,
		children
	}: {
		res: Resource<T>;
		/** When it returns true the empty state is shown instead of the content. */
		isEmpty?: (data: T) => boolean;
		children: Snippet<[T]>;
	} = $props();
</script>

{#if res.error !== null}
	<ErrorBox message={res.error} onretry={() => res.load()} />
{/if}
{#if res.data !== null}
	{#if isEmpty?.(res.data)}
		<div class="empty">아직 없음</div>
	{:else}
		{@render children(res.data)}
	{/if}
{:else if res.error === null}
	<div class="empty">불러오는 중</div>
{/if}
