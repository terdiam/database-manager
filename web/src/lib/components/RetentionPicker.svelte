<script lang="ts">
	import { date } from '#lib/format.ts';
	import { t } from '#lib/i18n/index.svelte.ts';
	import { expiresAt, type RetentionUnit } from '#lib/schedule.ts';

	let { unit = $bindable(), amount = $bindable() }: { unit: RetentionUnit; amount: number } = $props();
	let gfs = $state({ daily: 7, weekly: 4, monthly: 12 });
</script>

<fieldset class="mb-4 rounded-lg border border-line p-3">
	<legend class="px-1 text-sm font-semibold">{t('retention.title')}</legend>
	<div class="mb-3 flex flex-wrap gap-4 text-sm">
		{#each ['days', 'months', 'gfs'] as const as option}
			<label class="flex items-center gap-1.5"><input type="radio" name="retention" value={option} bind:group={unit} />{t(`retention.${option}`)}</label>
		{/each}
	</div>
	{#if unit === 'gfs'}
		<div class="grid gap-3 sm:grid-cols-3">
			{#each ['daily', 'weekly', 'monthly'] as const as key}
				<label class="text-sm">{t(`retention.keep.${key}`)}<input class="input mt-1" type="number" min="0" required bind:value={gfs[key]} /></label>
			{/each}
		</div>
	{:else}
		<label class="text-sm">{t('retention.amount')} ({t(`retention.${unit}`).toLowerCase()})
			<input class="input mt-1 max-w-40" type="number" min="1" max={unit === 'days' ? 3650 : 120} required bind:value={amount} />
		</label>
		{#if amount > 0}
			<p class="mt-2 text-xs text-dim">{t('retention.preview', { date: date(expiresAt(unit, amount)) })}</p>
		{/if}
	{/if}
	<p class="mt-1 text-xs text-dim">{t('retention.safety')}</p>
</fieldset>
