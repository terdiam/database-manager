<script lang="ts">
	import { resolve } from '$app/paths';
	import { engines, servers, snapshots } from '#lib/api/index.ts';
	import BrandIcon from '#lib/components/BrandIcon.svelte';
	import DateFilter from '#lib/components/DateFilter.svelte';
	import { emptyDate, inDate } from '#lib/daterange.ts';
	import DataTable from '#lib/components/DataTable.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import Select from '#lib/components/Select.svelte';
	import StatusBadge from '#lib/components/StatusBadge.svelte';
	import { bytes, date, dateTime } from '#lib/format.ts';
	import { t } from '#lib/i18n/index.svelte.ts';

	let server = $state('');
	let type = $state('');
	let period = $state(emptyDate());
	const rows = $derived(snapshots.filter((s) => (!server || s.server === server) && (!type || s.type === type) && inDate(s.created, period)));
</script>

<PageHeader title={t('nav.snapshots')} description={t('snapshots.desc')} />

<search class="mb-3 flex flex-wrap items-center gap-2">
	<DateFilter bind:value={period} />
	<Select class="w-56" aria-label={t('nav.databases')} bind:value={server} options={[{ value: '', label: t('filter.allServers'), icon: 'database' as const }, ...servers.map((s) => ({ value: s.name, label: s.name, brand: s.engine }))]} />
	<Select class="w-48" aria-label={t('col.type')} bind:value={type} options={[{ value: '', label: t('filter.allTypes') }, { value: 'full', label: t('type.full') }, { value: 'incremental', label: t('type.incremental') }]} />
</search>

<p class="mb-2 text-xs text-dim" role="status">{t('filter.count', { n: rows.length, total: snapshots.length })}</p>

<DataTable label={t('nav.snapshots')} columns={['ID', t('col.source'), t('col.type'), t('col.size'), t('col.created'), t('col.expires'), t('col.status')]} {rows}>
	{#snippet cell(item, i)}
		{#if i === 0}<a class="font-medium text-brand underline" href={resolve(`snapshots/${item.id}`)}>{item.id}</a>
		{:else if i === 1}<span class="inline-flex items-center gap-2"><BrandIcon name={item.engine} /><span>{item.server}<br /><span class="text-xs text-dim">{engines[item.engine].label} · {item.database}</span></span></span>
		{:else if i === 2}{t(`type.${item.type}`)}
		{:else if i === 3}{bytes(item.size)}
		{:else if i === 4}{dateTime(item.created)}
		{:else if i === 5}{date(item.expires)}
		{:else}<StatusBadge status={item.status} />{/if}
	{/snippet}
</DataTable>
