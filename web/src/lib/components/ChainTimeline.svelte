<script lang="ts">
	import { resolve } from '$app/paths';
	import type { Snapshot } from '#lib/api/index.ts';
	import { bytes, dateTime } from '#lib/format.ts';
	import { t } from '#lib/i18n/index.svelte.ts';
	import StatusBadge from './StatusBadge.svelte';

	let { items, current }: { items: Snapshot[]; current: string } = $props();
	const ordered = $derived([...items].sort((a, b) => a.created.localeCompare(b.created)));
</script>

<ol class="border-l-2 border-line pl-4">
	{#each ordered as item (item.id)}
		<li class="relative mb-3 last:mb-0">
			<span class="absolute top-1.5 -left-[1.45rem] size-3 rounded-full border-2 border-brand {item.type === 'full' ? 'bg-brand' : 'bg-surface'}"></span>
			<a href={resolve(`snapshots/${item.id}`)} aria-current={item.id === current ? 'page' : undefined} class="flex flex-wrap items-center gap-2 text-sm hover:underline {item.id === current ? 'font-semibold' : ''}">
				{t(`type.${item.type}`)} · {dateTime(item.created)} · {bytes(item.size)}
				<StatusBadge status={item.status} />
			</a>
		</li>
	{/each}
</ol>
