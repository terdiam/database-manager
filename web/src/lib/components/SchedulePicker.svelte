<script lang="ts">
	import { request } from '#lib/api/http.ts';
	import { dateTime } from '#lib/format.ts';
	import { locale, t } from '#lib/i18n/index.svelte.ts';
	import { toCron, type Schedule } from '#lib/schedule.ts';

	let { legend, value = $bindable(), timezone = $bindable() }: { legend: string; value: Schedule; timezone: string } = $props();
	const id = $props.id();
	const kinds: Schedule['kind'][] = ['manual', 'hours', 'daily', 'weekly', 'monthly', 'yearly', 'cron'];
	const zones = ['UTC', ...Intl.supportedValuesOf('timeZone')];
	const names = (unit: 'weekday' | 'month') =>
		Array.from({ length: unit === 'weekday' ? 7 : 12 }, (_, i) =>
			new Intl.DateTimeFormat(locale.current, { [unit]: 'long' }).format(unit === 'weekday' ? new Date(2026, 1, 1 + i) : new Date(2026, i, 1))
		);

	// The server computes the preview, so it matches what will really run.
	let runs = $state<string[]>([]);
	let invalid = $state(false);
	$effect(() => {
		const pattern = toCron(value);
		const query = pattern ? new URLSearchParams({ schedule: pattern, timezone }).toString() : '';
		runs = [];
		invalid = false;
		if (!query) return;
		let stale = false;
		const timer = setTimeout(() => {
			request<{ runs: string[] }>('GET', `/schedule/preview?${query}`).then(
				(reply) => !stale && (runs = reply.runs),
				() => !stale && (invalid = true)
			);
		}, 300);
		return () => {
			stale = true;
			clearTimeout(timer);
		};
	});
</script>

<fieldset class="mb-4 rounded-lg border border-line p-3">
	<legend class="px-1 text-sm font-semibold">{legend}</legend>
	<div class="grid gap-3 sm:grid-cols-3">
		<label class="text-sm">{t('schedule.frequency')}
			<select class="input mt-1" bind:value={value.kind}>
				{#each kinds as kind}<option value={kind}>{t(`schedule.${kind}`)}</option>{/each}
			</select>
		</label>

		{#if value.kind === 'hours'}
			<label class="text-sm">{t('schedule.every')}
				<input class="input mt-1" type="number" min="1" max="23" required bind:value={value.every} />
			</label>
		{/if}
		{#if value.kind === 'weekly'}
			<label class="text-sm">{t('schedule.weekday')}
				<select class="input mt-1" bind:value={value.weekday}>
					{#each names('weekday') as name, i}<option value={i}>{name}</option>{/each}
				</select>
			</label>
		{/if}
		{#if value.kind === 'yearly'}
			<label class="text-sm">{t('schedule.month')}
				<select class="input mt-1" bind:value={value.month}>
					{#each names('month') as name, i}<option value={i + 1}>{name}</option>{/each}
				</select>
			</label>
		{/if}
		{#if value.kind === 'monthly' || value.kind === 'yearly'}
			<label class="text-sm">{t('schedule.day')}
				<input class="input mt-1" type="number" min="1" max="31" required bind:value={value.day} />
			</label>
		{/if}
		{#if ['daily', 'weekly', 'monthly', 'yearly'].includes(value.kind)}
			<label class="text-sm">{t('schedule.time')}
				<input class="input mt-1" type="time" required bind:value={value.time} />
			</label>
		{/if}
		{#if value.kind === 'cron'}
			<label class="text-sm sm:col-span-2">{t('schedule.cron')}
				<input class="input mt-1 font-mono" required autocapitalize="none" spellcheck="false" bind:value={value.cron} aria-describedby="{id}-cron" />
				<span id="{id}-cron" class="text-xs text-dim">{t('schedule.cronHelp')}</span>
			</label>
		{/if}
		{#if value.kind !== 'manual'}
			<label class="text-sm">{t('schedule.timezone')}
				<select class="input mt-1" bind:value={timezone}>
					{#each zones as zone}<option value={zone}>{zone}</option>{/each}
				</select>
			</label>
		{/if}
	</div>
	<p class="mt-2 text-xs text-dim empty:hidden" role="status">
		{#if invalid}<span class="text-bad">{t('error.schedule')}</span>{:else if runs.length}{t('schedule.next')}: {runs.map(dateTime).join(' · ')}{/if}
	</p>
</fieldset>
