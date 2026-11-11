<script lang="ts">
	import { api, type Judgment, type Ledger, type LedgerHorizon } from '#lib/api.ts';
	import Loader from '#lib/components/Loader.svelte';
	import {
		BIAS_LABEL,
		REGIME_LABEL,
		SOURCE_LABEL,
		biasClass,
		fmtDateTime,
		fmtNum,
		fmtPct,
		fmtPrice,
		isNum,
		label,
		signClass
	} from '#lib/format.ts';
	import { Resource, loadOn } from '#lib/resource.svelte.ts';
	import { app } from '#lib/state.svelte.ts';

	const HISTORY_LIMIT = 100;

	const ledger = new Resource<Ledger>(() => api.ledger(app.asset));
	const history = new Resource<Judgment[]>(() => api.judgments(app.asset, HISTORY_LIMIT));

	loadOn(
		() => app.asset,
		() => {
			void ledger.load(true);
			void history.load(true);
		}
	);

	const sourceLabels = $derived(
		new Map((ledger.data?.sources ?? []).map((s) => [s.source as string, s.label]))
	);
</script>

{#snippet horizonCells(h: LedgerHorizon)}
	<td class="gap">{fmtNum(h.directional_n, 0)}</td>
	<td class="rate">{isNum(h.hit_rate) ? `${fmtNum(h.hit_rate * 100, 1)}%` : '-'}</td>
	<td class={signClass(h.avg_ret_up_pct)}>{fmtPct(h.avg_ret_up_pct)}</td>
	<td class={signClass(h.avg_ret_down_pct)}>{fmtPct(h.avg_ret_down_pct)}</td>
{/snippet}

{#snippet outcome(ret: number | null, hit: boolean | null)}
	{#if !isNum(ret)}
		<span class="muted">대기</span>
	{:else}
		<span class={signClass(ret)}>{fmtPct(ret)}</span>
		<span class={hit ? 'ok' : 'muted'}>{hit ? '적중' : '빗나감'}</span>
	{/if}
{/snippet}

<svelte:head><title>적중률 | Crypto News</title></svelte:head>

<div class="pagehead">
	<h1>{app.asset.toUpperCase()} 적중률</h1>
	<p class="muted">판정이 4시간, 24시간 뒤 실제 가격과 맞았는지 셉니다. 기준선보다 낮으면 믿을 이유가 없습니다.</p>
</div>

<section class="section">
	<Loader res={ledger} isEmpty={(l) => l.sources.length === 0}>
		{#snippet children(l)}
			<div class="table-wrap">
				<table>
					<thead>
						<tr class="groups">
							<th></th>
							<th colspan="4" class="gap">4시간 뒤</th>
							<th colspan="4" class="gap">24시간 뒤</th>
						</tr>
						<tr>
							<th>출처</th>
							{#each ['4h', '24h'] as h (h)}
								<th class="gap">방향 판정</th>
								<th>적중률</th>
								<th>상승 판정 뒤</th>
								<th>하락 판정 뒤</th>
							{/each}
						</tr>
					</thead>
					<tbody>
						{#each l.sources as s (s.source)}
							<tr class:baseline={s.source.startsWith('baseline_')}>
								<td>{s.label}</td>
								{@render horizonCells(s.h4)}
								{@render horizonCells(s.h24)}
							</tr>
						{/each}
					</tbody>
				</table>
			</div>
		{/snippet}
	</Loader>
</section>

{#if ledger.data && ledger.data.by_regime.length > 0}
	<section class="section">
		<h2>국면별 이후 수익률</h2>
		<div class="table-wrap">
			<table>
				<thead>
					<tr>
						<th>국면</th>
						<th class="l">출처</th>
						<th>횟수</th>
						<th>4시간 뒤 평균</th>
						<th>24시간 뒤 평균</th>
					</tr>
				</thead>
				<tbody>
					{#each ledger.data.by_regime as r (r.source + r.regime)}
						<tr>
							<td>{label(REGIME_LABEL, r.regime)}</td>
							<td class="l">{sourceLabels.get(r.source) ?? r.source}</td>
							<td>{fmtNum(r.n, 0)}</td>
							<td class={signClass(r.avg_ret_4h_pct)}>{fmtPct(r.avg_ret_4h_pct)}</td>
							<td class={signClass(r.avg_ret_24h_pct)}>{fmtPct(r.avg_ret_24h_pct)}</td>
						</tr>
					{/each}
				</tbody>
			</table>
		</div>
	</section>
{/if}

<section class="section">
	<h2>판정 기록</h2>
	<Loader res={history} isEmpty={(list) => list.length === 0}>
		{#snippet children(list)}
			<div class="table-wrap">
				<table>
					<thead>
						<tr>
							<th>시각</th>
							<th class="l">출처</th>
							<th class="l">국면</th>
							<th class="l">4시간</th>
							<th class="l">24시간</th>
							<th>당시 가격</th>
							<th>4시간 뒤</th>
							<th>24시간 뒤</th>
						</tr>
					</thead>
					<tbody>
						{#each list as j (j.id)}
							<tr>
								<td>{fmtDateTime(j.ts)}</td>
								<td class="l">{label(SOURCE_LABEL, j.source)}</td>
								<td class="l">{label(REGIME_LABEL, j.regime)}</td>
								<td class="l {biasClass(j.bias_4h)}">{label(BIAS_LABEL, j.bias_4h)}</td>
								<td class="l {biasClass(j.bias_24h)}">{label(BIAS_LABEL, j.bias_24h)}</td>
								<td>{fmtPrice(j.price)}</td>
								<td class="out">{@render outcome(j.ret_4h_pct, j.hit_4h)}</td>
								<td class="out">{@render outcome(j.ret_24h_pct, j.hit_24h)}</td>
							</tr>
						{/each}
					</tbody>
				</table>
			</div>
		{/snippet}
	</Loader>
</section>

<style>
	.pagehead p {
		margin-top: 6px;
		font-size: 14px;
	}
	h2 {
		margin-bottom: 14px;
	}
	.groups th {
		padding-bottom: 2px;
		border-bottom: 0;
		color: var(--ink);
		font-size: 14px;
		font-weight: 600;
		text-align: left;
	}
	.gap {
		padding-left: 44px;
	}
	.l {
		text-align: left;
	}
	.rate {
		font-weight: 700;
	}
	.baseline td {
		color: var(--ink-3);
	}
	.out :global(span + span) {
		margin-left: 8px;
	}
	.ok {
		font-weight: 600;
	}
</style>
