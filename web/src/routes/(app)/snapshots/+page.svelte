<script lang="ts">
	import IconButton from '#lib/components/IconButton.svelte';
	import { onMount } from 'svelte';
	import { resolve } from '$app/paths';
	import { request, type DbServer, type Page, type Snapshot } from '#lib/api/http.ts';
	import DataTable from '#lib/components/DataTable.svelte';
	import DateFilter from '#lib/components/DateFilter.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import Select from '#lib/components/Select.svelte';
	import { dateParams, emptyDate } from '#lib/daterange.ts';
	import { errorText } from '#lib/errors.ts';
	import { bytes, dateTime } from '#lib/format.ts';
	import { t } from '#lib/i18n/index.svelte.ts';
	import { live } from '#lib/events.ts';

	let period = $state(emptyDate());
	let server = $state('');
	let servers = $state<DbServer[]>([]);
	let rows = $state<Snapshot[]>([]);
	let next = $state<string | null>(null);
	let error = $state('');
	let paged = false; // older pages are on screen; a refresh would drop them

	const loadServers = () => request<DbServer[]>('GET', '/database-servers').then((list) => (servers = list), (e) => void (error = errorText(e)));
	loadServers();

	const filter = $derived.by(() => {
		const params = dateParams(period);
		if (server) params.set('server', server);
		return params.toString();
	});

	async function load(query: string, before: string | null) {
		paged = before !== null;
		try {
			const page = await request<Page<Snapshot>>('GET', `/snapshots?${query}${before ? `&before=${before}` : ''}`);
			if (query !== filter) return;
			rows = before ? [...rows, ...page.items] : page.items;
			next = page.next;
			error = '';
		} catch (e) {
			error = errorText(e);
		}
	}
	$effect(() => void load(filter, null));
	onMount(() => live(['snapshots', 'database_servers'], () => paged || Promise.all([load(filter, null), loadServers()])));
</script>

<PageHeader title={t('nav.snapshots')} description={t('snapshots.desc')} />
<p class="mb-3 text-sm text-bad empty:hidden" role="alert">{error}</p>

<search class="mb-3 flex flex-wrap items-center gap-2">
	<DateFilter bind:value={period} />
	<Select class="w-56" aria-label={t('nav.databases')} bind:value={server} options={[{ value: '', label: t('filter.allServers'), icon: 'database' as const }, ...servers.map((s) => ({ value: s.name, label: s.name, brand: s.engine }))]} />
</search>

<DataTable label={t('nav.snapshots')} columns={[t('col.created'), t('col.source'), t('col.destination'), t('col.size'), t('col.expires'), t('col.actions')]} {rows}>
	{#snippet cell(s, i)}
		{#if i === 0}<span class="whitespace-nowrap">{dateTime(s.created_at)}</span>
		{:else if i === 1}<strong>{s.server_name}</strong> / {s.database_name}
		{:else if i === 2}{s.destination}
		{:else if i === 3}{bytes(s.size)}
		{:else if i === 4}{s.expires_at ? dateTime(s.expires_at) : '-'}
		{:else}<IconButton icon="eye" label={t('snapshots.open')} href={resolve(`snapshots/${s.id}`)} />{/if}
	{/snippet}
</DataTable>

{#if next}<button type="button" class="btn mt-3" onclick={() => load(filter, next)}>{t('action.more')}</button>{/if}
