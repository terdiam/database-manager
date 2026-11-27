<script lang="ts" module>
	export type Series = 'completed' | 'failed';
</script>

<script lang="ts">
	import { date } from '#lib/format.ts';
	import { t } from '#lib/i18n/index.svelte.ts';
	import Icon from './Icon.svelte';
	import SplitTitle from './SplitTitle.svelte';

	/** `data`: [succeeded, failed] per day, oldest first. `filter` is the colour picked by the user. */
	let { data, filter = $bindable('') }: { data: number[][]; filter?: Series | '' } = $props();

	// Drawn top to bottom; failures sit on top where they stand out.
	const series: { key: Series; index: number; color: string }[] = [
		{ key: 'failed', index: 1, color: 'bg-bad' },
		{ key: 'completed', index: 0, color: 'bg-brand' }
	];
	const shown = $derived(series.filter((s) => !filter || s.key === filter));
	// The scale follows the filter, so a lone small series is still readable.
	const peak = $derived(Math.max(1, ...data.map((day) => shown.reduce((sum, s) => sum + day[s.index], 0))));
	const total = (index: number) => data.reduce((sum, day) => sum + day[index], 0);
	const day = (i: number) => date(new Date(Date.now() - (data.length - 1 - i) * 86_400_000));

	let hoverDay = $state(-1);
	let hoverSeries = $state<Series | ''>('');
	const toggle = (key: Series) => (filter = filter === key ? '' : key);
</script>

<section class="card">
	<div class="mb-3 flex flex-wrap items-baseline justify-between gap-2">
		<SplitTitle text={t('dashboard.activity')} class="font-semibold" />
		<p class="text-xs text-dim tabular-nums">
			{#if hoverDay >= 0}
				<strong class="text-ink">{day(hoverDay)}</strong>
				{#each shown as s} · {t(`status.${s.key}`)} {data[hoverDay][s.index]}{/each}
			{:else}
				{t('chart.hint')}
			{/if}
		</p>
	</div>

	<!-- Pointer shortcut only: keyboard and screen reader users filter with the legend buttons below. -->
	<div class="flex h-40 items-stretch gap-1 border-b border-line" aria-hidden="true">
		{#each data as values, i}
			<div class="group flex min-w-0 flex-1 flex-col justify-end rounded-t transition-colors duration-200 hover:bg-muted" role="presentation" onpointerenter={() => (hoverDay = i)} onpointerleave={() => (hoverDay = -1)}>
				{#each series as s (s.key)}
					{@const visible = !filter || filter === s.key}
					{@const dimmed = (hoverSeries && hoverSeries !== s.key) || (hoverDay >= 0 && hoverDay !== i)}
					<button
						type="button"
						tabindex="-1"
						class="w-full shrink-0 origin-bottom transition-all duration-300 ease-out first:rounded-t hover:scale-x-110 hover:brightness-125 hover:saturate-150 motion-reduce:transition-none {s.color} {dimmed ? 'opacity-35' : ''}"
						style:height="{visible ? (values[s.index] / peak) * 100 : 0}%"
						style:min-height={visible && values[s.index] ? '6px' : '0'}
						title="{day(i)} · {t(`status.${s.key}`)}: {values[s.index]}"
						onclick={() => toggle(s.key)}
						onpointerenter={() => (hoverSeries = s.key)}
						onpointerleave={() => (hoverSeries = '')}
					></button>
				{/each}
			</div>
		{/each}
	</div>
	<div class="mt-1 flex justify-between text-xs text-dim" aria-hidden="true"><span>{day(0)}</span><span>{t('chart.max', { n: peak })}</span><span>{day(data.length - 1)}</span></div>

	<div class="mt-2 flex flex-wrap gap-2" role="group" aria-label={t('chart.filter')}>
		{#each [series[1], series[0]] as s (s.key)}
			<button type="button" class="btn min-h-8 text-xs {filter === s.key ? 'border-brand bg-muted' : filter ? 'opacity-50' : ''}" aria-pressed={filter === s.key} onclick={() => toggle(s.key)} onpointerenter={() => (hoverSeries = s.key)} onpointerleave={() => (hoverSeries = '')}>
				<span class="size-3 rounded-sm {s.color}" aria-hidden="true"></span>
				{t(`status.${s.key}`)} <strong class="tabular-nums">{total(s.index)}</strong>
			</button>
		{/each}
		{#if filter}
			<button type="button" class="btn min-h-8 text-xs" onclick={() => (filter = '')}><Icon name="close" size={14} />{t('chart.all')}</button>
		{/if}
	</div>

	<table class="sr-only">
		<caption>{t('dashboard.activity')}</caption>
		<tbody>{#each data as [good, failed], i}<tr><th>{day(i)}</th><td>{good} {t('status.completed')}</td><td>{failed} {t('status.failed')}</td></tr>{/each}</tbody>
	</table>
</section>
