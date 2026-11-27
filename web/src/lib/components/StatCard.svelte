<script lang="ts">
	import { countUp } from '#lib/motion.ts';
	import Icon, { type IconName } from './Icon.svelte';
	let { label, value, hint = '', icon, tone = '' }: { label: string; value: string | number; hint?: string; icon: IconName; tone?: string } = $props();
</script>

<div class="card">
	<div class="flex items-center justify-between text-sm text-dim">{label}<Icon name={icon} class={tone} /></div>
	{#if typeof value === 'number'}
		<p class="mt-2 text-2xl font-semibold tabular-nums {tone}" {@attach countUp(() => value)}></p>
	{:else}
		<p class="mt-2 text-2xl font-semibold tabular-nums {tone}">{value}</p>
	{/if}
	<p class="text-xs text-dim">{hint}</p>
</div>
