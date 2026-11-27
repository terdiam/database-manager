<script lang="ts">
	import { t } from '#lib/i18n/index.svelte.ts';
	import Illustration from './Illustration.svelte';
	import Modal from './Modal.svelte';

	const steps = ['database', 'destination', 'backup', 'monitor'] as const;
	let step = $state(0);
	let modal: Modal;

	export function open() {
		step = 0;
		modal.open();
	}
</script>

<Modal bind:this={modal} title={t('menu.tour')}>
	{#snippet children(close)}
		<Illustration name="backup" class="mx-auto mb-3 w-56" />
		<div aria-live="polite">
			<p class="text-xs text-dim">{step + 1} / {steps.length}</p>
			<h3 class="font-semibold">{t(`tour.${steps[step]}.title`)}</h3>
			<p class="mt-1 min-h-10 text-sm">{t(`tour.${steps[step]}.text`)}</p>
		</div>
		<div class="mt-4 flex justify-between">
			<button type="button" class="btn" disabled={step === 0} onclick={() => (step -= 1)}>{t('action.back')}</button>
			<button type="button" class="btn btn-primary" onclick={() => (step < steps.length - 1 ? (step += 1) : close())}>
				{step < steps.length - 1 ? t('action.next') : t('action.done')}
			</button>
		</div>
	{/snippet}
</Modal>
