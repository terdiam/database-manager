<script lang="ts">
	import { date } from '#lib/format.ts';
	import { t } from '#lib/i18n/index.svelte.ts';
	import { expiresAt, type Retention } from '#lib/schedule.ts';

	let { value = $bindable() }: { value: Retention } = $props();
	const id = $props.id();
</script>

<fieldset class="mb-4 rounded-lg border border-line p-3">
	<legend class="px-1 text-sm font-semibold">{t('retention.title')}</legend>
	<div class="mb-3 flex flex-wrap gap-4 text-sm">
		{#each ['days', 'months', 'gfs', 'all'] as const as option}
			<label class="flex items-center gap-1.5"><input type="radio" name="{id}-retention" value={option} bind:group={value.unit} />{t(`retention.${option}`)}</label>
		{/each}
	</div>
	{#if value.unit === 'gfs'}
		<div class="grid gap-3 sm:grid-cols-3">
			{#each ['daily', 'weekly', 'monthly'] as const as key}
				<label class="text-sm">{t(`retention.keep.${key}`)}<input class="input mt-1" type="number" min="0" max="1000" required bind:value={value[key]} /></label>
			{/each}
		</div>
	{:else if value.unit !== 'all'}
		<label class="text-sm">{t('retention.amount')} ({t(`retention.${value.unit}`).toLowerCase()})
			<input class="input mt-1 max-w-40" type="number" min="1" max={value.unit === 'days' ? 3650 : 120} required bind:value={value.amount} />
		</label>
		{#if value.amount > 0}
			<p class="mt-2 text-xs text-dim">{t('retention.preview', { date: date(expiresAt(value.unit, value.amount)) })}</p>
		{/if}
	{/if}
	<p class="mt-1 text-xs text-dim">{t('retention.safety')}</p>
</fieldset>
