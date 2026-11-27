<script lang="ts" module>
	import type { IconName } from './Icon.svelte';
	export type Option = { value: string; label: string; description?: string; brand?: string; icon?: IconName };
</script>

<script lang="ts">
	import type { HTMLSelectAttributes } from 'svelte/elements';
	import BrandIcon from './BrandIcon.svelte';
	import Icon from './Icon.svelte';

	// Native <select> with logos and descriptions. Browsers without customizable select show the text only.
	let {
		value = $bindable(),
		options,
		placeholder = '',
		class: cls = '',
		...rest
	}: Omit<HTMLSelectAttributes, 'value'> & { value?: string; options: Option[]; placeholder?: string } = $props();
</script>

<select {...rest} class="input {cls}" bind:value>
	<button><selectedcontent></selectedcontent></button>
	{#if placeholder}<option value="" disabled>{placeholder}</option>{/if}
	{#each options as option (option.value)}
		<option value={option.value}>
			{#if option.brand}<BrandIcon name={option.brand} size={16} />{/if}
			{#if option.icon}<Icon name={option.icon} class="text-dim" />{/if}
			<span class="min-w-0">
				<span class="block truncate">{option.label}</span>
				{#if option.description}<span class="opt-desc text-xs font-normal text-dim"><span class="sr-only"> - </span>{option.description}</span>{/if}
			</span>
		</option>
	{/each}
</select>
