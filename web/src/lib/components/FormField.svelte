<script lang="ts">
	import type { Snippet } from 'svelte';
	let { label, help = '', children }: { label: string; help?: string; children: Snippet<[{ id: string; 'aria-describedby': string | undefined }]> } = $props();
	const id = $props.id();
</script>

<div class="mb-3">
	<label for={id} class="mb-1 block text-sm font-medium">{label}</label>
	{@render children({ id, 'aria-describedby': help ? `${id}-help` : undefined })}
	{#if help}<p id="{id}-help" class="mt-1 text-xs text-dim">{help}</p>{/if}
</div>
