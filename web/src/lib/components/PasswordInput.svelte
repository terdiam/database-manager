<script lang="ts">
	import type { HTMLInputAttributes } from 'svelte/elements';
	import { t } from '#lib/i18n/index.svelte.ts';
	import Icon from './Icon.svelte';

	let {
		value = $bindable(),
		element = $bindable(),
		class: cls = '',
		...rest
	}: Omit<HTMLInputAttributes, 'type' | 'value'> & { value: string; element?: HTMLInputElement } = $props();
	let shown = $state(false);
</script>

<span class="relative block {cls}">
	<input {...rest} bind:this={element} class="input min-h-11 pr-11" type={shown ? 'text' : 'password'} autocapitalize="off" spellcheck="false" bind:value />
	<button type="button" class="absolute inset-y-0 right-0 grid w-11 place-items-center rounded-r-lg text-dim hover:text-ink" aria-pressed={shown} aria-label={t('password.show')} onclick={() => (shown = !shown)}>
		<Icon name={shown ? 'eyeOff' : 'eye'} />
	</button>
</span>
