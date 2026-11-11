<script lang="ts">
	let { message, onretry }: { message: string; onretry: () => void } = $props();
</script>

<div class="error" role="alert">
	<span class="hot">불러오지 못함: {message}</span>
	<button class="btn" type="button" onclick={onretry}>다시 시도</button>
</div>

<style>
	.error {
		display: flex;
		align-items: center;
		gap: 14px;
		flex-wrap: wrap;
		padding: 10px 0;
		font-size: 14px;
	}
</style>
