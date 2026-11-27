<script lang="ts">
	import { t } from '#lib/i18n/index.svelte.ts';
	import Modal from './Modal.svelte';

	let { title, message, confirmLabel, expect = '', onconfirm }: { title: string; message: string; confirmLabel: string; expect?: string; onconfirm: () => void } = $props();
	let modal: Modal;
	let typed = $state('');
	const id = $props.id();

	export function open() {
		typed = '';
		modal.open();
	}
</script>

<Modal bind:this={modal} {title}>
	{#snippet children(close)}
		<p class="text-sm">{message}</p>
		{#if expect}
			<label for={id} class="mt-3 mb-1 block text-sm">{t('confirm.type', { name: expect })}</label>
			<input {id} class="input" autocomplete="off" bind:value={typed} />
		{/if}
		<div class="mt-5 flex justify-end gap-2">
			<!-- svelte-ignore a11y_autofocus (safe action gets focus in a destructive dialog) -->
			<button type="button" class="btn" autofocus onclick={close}>{t('action.cancel')}</button>
			<button type="button" class="btn btn-danger" disabled={typed !== expect} onclick={() => { close(); onconfirm(); }}>{confirmLabel}</button>
		</div>
	{/snippet}
</Modal>
