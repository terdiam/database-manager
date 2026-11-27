<script lang="ts">
	import SplitTitle from '#lib/components/SplitTitle.svelte';
	import type { Snippet } from 'svelte';
	let { title, description = '', actions }: { title: string; description?: string; actions?: Snippet } = $props();
</script>

<svelte:head><title>{title} · DB Manager</title></svelte:head>

<header class="mb-5 flex flex-wrap items-end justify-between gap-3">
	<div>
		<SplitTitle level="h1" text={title} class="text-2xl font-bold tracking-tight" />
		{#if description}<p class="mt-1 max-w-2xl text-sm text-dim">{description}</p>{/if}
	</div>
	{#if actions}<div class="flex flex-wrap gap-2">{@render actions()}</div>{/if}
</header>
