<script lang="ts">
	import { flip } from 'svelte/animate';
	import { cubicOut } from 'svelte/easing';
	import { prefersReducedMotion } from 'svelte/motion';
	import { fly, scale } from 'svelte/transition';
	import { t } from '#lib/i18n/index.svelte.ts';
	import Icon from './Icon.svelte';
	import { dismiss, toasts } from './toast.svelte';

	const time = (ms: number) => (prefersReducedMotion.current ? 0 : ms);
</script>

<!-- Live region stays mounted so additions are announced. Each toast closes itself when its bar
     runs out; hovering or focusing it holds the bar, and with it the timer. -->
<div class="fixed right-4 bottom-4 z-50 flex w-[min(22rem,calc(100vw-2rem))] flex-col gap-2" role="status" aria-label={t('toast.region')}>
	{#each toasts as item (item.id)}
		<div
			class="toast card relative flex items-start gap-2 overflow-hidden shadow-lg"
			in:fly={{ x: 64, duration: time(300), easing: cubicOut }}
			out:scale={{ start: 0.92, duration: time(180), easing: cubicOut }}
			animate:flip={{ duration: time(220), easing: cubicOut }}
		>
			<span class="toast-icon mt-0.5 text-ok"><Icon name="check" /></span>
			<p class="flex-1 text-sm">{item.text}</p>
			<button type="button" aria-label={t('action.close')} onclick={() => dismiss(item.id)}><Icon name="close" size={16} /></button>
			<span class="toast-timer" aria-hidden="true" onanimationend={() => dismiss(item.id)}></span>
		</div>
	{/each}
</div>
