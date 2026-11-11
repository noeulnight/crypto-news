<script lang="ts">
	import '../app.css';
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import favicon from '#lib/assets/favicon.svg';
	import { ASSETS } from '#lib/api.ts';
	import AlertList from '#lib/components/AlertList.svelte';
	import ErrorBox from '#lib/components/ErrorBox.svelte';
	import Loader from '#lib/components/Loader.svelte';
	import { fmtPct, fmtPrice, signClass } from '#lib/format.ts';
	import { loadOn } from '#lib/resource.svelte.ts';
	import { app } from '#lib/state.svelte.ts';
	import type { LayoutProps } from './$types';

	let { children }: LayoutProps = $props();

	const NAV = [
		{ href: '/', label: '국면' },
		{ href: '/indicators', label: '지표' },
		{ href: '/news', label: '속보' },
		{ href: '/calendar', label: '일정' },
		{ href: '/ledger', label: '적중률' }
	];
	const NOTIFY_LABEL = {
		default: '브라우저 알림 켜기',
		granted: '브라우저 알림 켜짐',
		denied: '브라우저 알림 차단됨',
		unsupported: '브라우저 알림 미지원'
	} as const;

	let panelOpen = $state(false);

	onMount(() => app.start());
	loadOn(
		() => app.asset,
		() => void app.overview.load(true)
	);

	function togglePanel(open: boolean) {
		panelOpen = open;
		// seen on open, and again on close for alerts that streamed in while it was open
		app.markAlertsSeen();
	}

	/** Collectors that failed, never ran, or are more than two cycles late. */
	const late = $derived(
		(app.status.data?.collectors ?? []).filter(
			(c) =>
				c.last_err !== null ||
				c.last_ok_ts === null ||
				(c.every_secs !== null && app.now - c.last_ok_ts > 2 * c.every_secs + 60)
		)
	);
</script>

<svelte:head>
	<link rel="icon" href={favicon} />
</svelte:head>

<svelte:window onkeydown={(e) => e.key === 'Escape' && panelOpen && togglePanel(false)} />

<div class="shell">
	<header>
		<div class="bar">
			<a class="brand" href="/">Crypto News</a>

			<nav aria-label="주 메뉴">
				{#each NAV as n (n.href)}
					<a href={n.href} aria-current={page.url.pathname === n.href ? 'page' : undefined}>{n.label}</a>
				{/each}
			</nav>

			<div class="right">
				<div class="switch" role="group" aria-label="자산 선택">
					{#each ASSETS as a (a)}
						<button type="button" aria-pressed={app.asset === a} onclick={() => app.setAsset(a)}>
							{a.toUpperCase()}
						</button>
					{/each}
				</div>
				<div class="quote">
					<span class="price">{fmtPrice(app.price)}</span>
					<span class={signClass(app.current?.chg_24h_pct)} title="24시간 변화">
						{fmtPct(app.current?.chg_24h_pct)}
					</span>
				</div>
				<button
					class="alerts"
					type="button"
					aria-label="알림, 안 읽은 {app.unread}건"
					aria-expanded={panelOpen}
					onclick={() => togglePanel(!panelOpen)}
				>
					알림{#if app.unread > 0}<span class="hot">{app.unread > 99 ? '99+' : app.unread}</span>{/if}
				</button>
			</div>
		</div>
	</header>

	<main>
		{@render children()}
	</main>

	<footer>
		{#if app.status.error !== null}
			<ErrorBox message={app.status.error} onretry={() => app.status.load()} />
		{/if}
		{#if app.status.data}
			{@const st = app.status.data}
			<span class:hot={late.length > 0}>
				{#if st.collectors.length === 0}
					수집 준비 중
				{:else if late.length === 0}
					수집 정상
				{:else}
					수집 지연: {late.map((c) => c.name).join(', ')}
				{/if}
			</span>
			{#if !app.streamConnected}<span class="hot">실시간 연결 끊김</span>{/if}
			<span>Claude {st.claude_enabled ? '켜짐' : '꺼짐'}</span>
			<span>텔레그램 {st.telegram_enabled ? '켜짐' : '꺼짐'}</span>
		{/if}
	</footer>
</div>

{#if panelOpen}
	<button class="backdrop" type="button" aria-label="알림 닫기" onclick={() => togglePanel(false)}></button>
	<aside aria-label="알림">
		<div class="panel-head">
			<h2>알림</h2>
			<button class="link" type="button" onclick={() => togglePanel(false)}>닫기</button>
		</div>
		<button
			class="link notify"
			type="button"
			disabled={app.notify !== 'default'}
			onclick={() => app.enableNotifications()}
		>
			{NOTIFY_LABEL[app.notify]}
		</button>
		<div class="panel-body">
			<Loader res={app.alerts} isEmpty={(list) => list.length === 0}>
				{#snippet children(list)}
					<AlertList alerts={list} />
				{/snippet}
			</Loader>
		</div>
	</aside>
{/if}

<style>
	.shell {
		min-height: 100dvh;
		display: flex;
		flex-direction: column;
	}
	header {
		position: sticky;
		top: 0;
		z-index: 10;
		background: var(--ground);
		border-bottom: 1px solid var(--line);
	}
	.bar {
		max-width: var(--w-wide);
		min-height: 64px;
		margin: 0 auto;
		padding: 0 24px;
		display: flex;
		align-items: center;
		gap: 8px 32px;
		flex-wrap: wrap;
	}
	.brand {
		font-size: 19px;
		font-weight: 800;
		letter-spacing: -0.04em;
		text-decoration: none;
		white-space: nowrap;
	}
	nav {
		display: flex;
		gap: 24px;
	}
	nav a {
		color: var(--ink-3);
		font-size: 16px;
		font-weight: 600;
		text-decoration: none;
		white-space: nowrap;
	}
	nav a:hover {
		color: var(--ink-2);
	}
	nav a[aria-current='page'] {
		color: var(--ink);
	}
	.right {
		margin-left: auto;
		display: flex;
		align-items: baseline;
		gap: 22px;
		font-size: 15px;
		font-weight: 600;
	}
	.quote {
		display: flex;
		align-items: baseline;
		gap: 8px;
		white-space: nowrap;
	}
	.price {
		font-weight: 700;
	}
	.alerts {
		display: inline-flex;
		gap: 5px;
		font-weight: 600;
		white-space: nowrap;
	}
	.alerts:hover {
		color: var(--ink-2);
	}
	main {
		flex: 1;
		width: 100%;
		max-width: var(--w-wide);
		margin: 0 auto;
		padding: 44px 24px 96px;
		min-width: 0;
	}
	footer {
		width: 100%;
		max-width: var(--w-wide);
		margin: 0 auto;
		padding: 18px 24px 36px;
		display: flex;
		gap: 4px 20px;
		flex-wrap: wrap;
		color: var(--ink-3);
		font-size: 13px;
	}
	.backdrop {
		position: fixed;
		inset: 0;
		z-index: 20;
		background: rgb(23 24 28 / 0.3);
		cursor: default;
	}
	aside {
		position: fixed;
		top: 0;
		right: 0;
		bottom: 0;
		z-index: 21;
		width: min(420px, 100vw);
		display: flex;
		flex-direction: column;
		padding: 22px 24px;
		background: var(--ground);
		/* The only thing that floats above the page. */
		box-shadow: -12px 0 40px rgb(23 24 28 / 0.12);
	}
	.panel-head {
		display: flex;
		align-items: baseline;
		justify-content: space-between;
	}
	.notify {
		align-self: flex-start;
		margin: 6px 0 10px;
		font-size: 13px;
	}
	.notify:disabled {
		color: var(--ink-3);
		cursor: default;
	}
	.panel-body {
		flex: 1;
		overflow-y: auto;
		min-height: 0;
	}

	@media (max-width: 760px) {
		.bar {
			padding: 12px 16px 0;
			gap: 10px 20px;
		}
		nav {
			order: 3;
			width: 100%;
			gap: 20px;
			padding-bottom: 10px;
			overflow-x: auto;
		}
		.right {
			gap: 14px;
		}
		main {
			padding: 28px 16px 72px;
		}
		footer {
			padding: 16px 16px 28px;
		}
	}
</style>
