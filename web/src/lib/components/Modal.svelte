<script lang="ts">
	import type { Snippet } from 'svelte';
	import { t } from '#lib/i18n/index.svelte.ts';
	import Icon from './Icon.svelte';

	let { title, children }: { title: string; children: Snippet<[() => void]> } = $props();
	const id = $props.id();
	let dialog: HTMLDialogElement;

	export const open = () => dialog.showModal();
	export const close = () => dialog.close();
</script>

<dialog bind:this={dialog} class="modal" aria-labelledby={id} {...{ closedby: 'closerequest' }}>
	<div class="mb-4 flex items-center justify-between gap-3">
		<h2 {id} class="text-lg font-semibold">{title}</h2>
		<button type="button" class="btn px-2" aria-label={t('action.close')} onclick={close}><Icon name="close" /></button>
	</div>
	{@render children(close)}
</dialog>
