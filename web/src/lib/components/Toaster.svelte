<script lang="ts">
	import { t } from '#lib/i18n/index.svelte.ts';
	import Icon from './Icon.svelte';
	import { dismiss, toasts } from './toast.svelte';
</script>

<!-- Live region stays mounted so additions are announced. Toasts stay until closed. -->
<div class="fixed right-4 bottom-4 z-50 flex w-[min(22rem,calc(100vw-2rem))] flex-col gap-2" role="status" aria-label={t('toast.region')}>
	{#each toasts as item (item.id)}
		<div class="card flex items-start gap-2 shadow-lg">
			<Icon name="check" class="mt-0.5 text-ok" />
			<p class="flex-1 text-sm">{item.text}</p>
			<button type="button" aria-label={t('action.close')} onclick={() => dismiss(item.id)}><Icon name="close" size={16} /></button>
		</div>
	{/each}
</div>
