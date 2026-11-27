<script lang="ts">
	import { emptyDate, presets, type DateValue } from '#lib/daterange.ts';
	import { t } from '#lib/i18n/index.svelte.ts';
	import Select from './Select.svelte';

	let { value = $bindable() }: { value: DateValue } = $props();
	const inverted = $derived(value.preset === 'range' && value.from && value.to && value.from > value.to);
</script>

<fieldset class="contents">
	<legend class="sr-only">{t('date.label')}</legend>
	<Select class="w-48" aria-label={t('date.label')} bind:value={value.preset} options={presets.map((p) => ({ value: p, label: t(`date.${p}`), icon: 'calendar' as const }))} />
	{#if value.preset === 'date'}
		<input type="date" class="input min-h-11 w-auto" aria-label={t('date.date')} bind:value={value.date} />
	{:else if value.preset === 'range'}
		<label class="flex items-center gap-1.5 text-sm">{t('date.from')}<input type="date" class="input min-h-11 w-auto" max={value.to || undefined} bind:value={value.from} /></label>
		<label class="flex items-center gap-1.5 text-sm">{t('date.to')}<input type="date" class="input min-h-11 w-auto" min={value.from || undefined} bind:value={value.to} /></label>
	{/if}
	{#if value.preset !== 'all'}
		<button type="button" class="btn min-h-11" onclick={() => (value = emptyDate())}>{t('date.reset')}</button>
	{/if}
	{#if inverted}<p class="w-full text-sm text-bad" role="alert">{t('date.inverted')}</p>{/if}
</fieldset>
