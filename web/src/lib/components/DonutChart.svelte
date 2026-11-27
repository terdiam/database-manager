<script lang="ts">
	import SplitTitle from '#lib/components/SplitTitle.svelte';
	import { bytes } from '#lib/format.ts';
	import { t } from '#lib/i18n/index.svelte.ts';

	let { title, items }: { title: string; items: { name: string; value: number }[] } = $props();

	// Validated categorical slots (adjacent pairs, light and dark). Beyond four, the rest folds into "Other".
	const colors = ['light-dark(#2a78d6,#3987e5)', 'light-dark(#eb6834,#d95926)', 'light-dark(#1baf7a,#199e70)', 'light-dark(#eda100,#c98500)'];
	const total = $derived(items.reduce((sum, i) => sum + i.value, 0));
	const slices = $derived.by(() => {
		const shown = items.length > 4
			? [...items.slice(0, 3), { name: t('chart.other'), value: items.slice(3).reduce((s, i) => s + i.value, 0) }]
			: items;
		let start = 0;
		return shown.map((item, i) => {
			const percent = total ? (item.value / total) * 100 : 0;
			const slice = { ...item, percent, start, color: colors[i] };
			start += percent;
			return slice;
		});
	});
	let active = $state('');
	const focus = $derived(slices.find((s) => s.name === active));
</script>

<section class="card @container">
	<SplitTitle text={title} class="mb-3 font-semibold" />
	<div class="flex flex-col items-center gap-4 @sm:flex-row">
		<svg viewBox="0 0 42 42" class="size-40 shrink-0" role="img" aria-label={title}>
			{#each slices as s (s.name)}
				<!-- r = 100 / 2π, so dash units are percentages; the 0.6 shortfall is the surface gap. -->
				<circle cx="21" cy="21" r="15.9155" fill="none" stroke-width={active === s.name ? 8 : 6.5} stroke={s.color}
					stroke-dasharray="{Math.max(s.percent - 0.6, 0)} {100 - Math.max(s.percent - 0.6, 0)}" stroke-dashoffset={25 - s.start}
					role="presentation" onpointerenter={() => (active = s.name)} onpointerleave={() => (active = '')}>
					<title>{s.name}: {bytes(s.value)} ({s.percent.toFixed(1)}%)</title>
				</circle>
			{/each}
			<text x="21" y="21" text-anchor="middle" class="fill-ink text-[4.5px] font-semibold">{bytes(focus?.value ?? total)}</text>
			<text x="21" y="26" text-anchor="middle" class="fill-dim text-[2.8px]">{focus ? `${focus.percent.toFixed(1)}%` : t('chart.total')}</text>
		</svg>
		<ul class="w-full min-w-0 text-sm">
			{#each slices as s (s.name)}
				<li class="flex items-center gap-2 rounded px-1 py-1 {active === s.name ? 'bg-muted' : ''}" onpointerenter={() => (active = s.name)} onpointerleave={() => (active = '')}>
					<span class="size-3 shrink-0 rounded-sm" style:background={s.color} aria-hidden="true"></span>
					<span class="min-w-0 flex-1 truncate">{s.name}</span>
					<span class="tabular-nums">{bytes(s.value)}</span>
					<span class="w-12 text-right text-dim tabular-nums">{s.percent.toFixed(1)}%</span>
				</li>
			{/each}
		</ul>
	</div>
</section>
