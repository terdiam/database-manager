<script lang="ts">
	import { resolve } from '$app/paths';
	import { engines, servers } from '#lib/api/index.ts';
	import BrandIcon from '#lib/components/BrandIcon.svelte';
	import DataTable from '#lib/components/DataTable.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import StatusBadge from '#lib/components/StatusBadge.svelte';
	import { toast } from '#lib/components/toast.svelte.ts';
	import { t } from '#lib/i18n/index.svelte.ts';

	let query = $state('');
	const rows = $derived(servers.filter((s) => `${s.name} ${s.engine}`.includes(query.toLowerCase())));
</script>

<PageHeader title={t('nav.databases')} description={t('databases.desc')}>
	{#snippet actions()}<a class="btn btn-primary" href={resolve('databases/new')}>{t('databases.new')}</a>{/snippet}
</PageHeader>

<search class="mb-3 block max-w-xs">
	<input class="input" type="search" aria-label={t('action.search')} placeholder={t('action.search')} bind:value={query} />
</search>

<DataTable label={t('nav.databases')} columns={[t('col.name'), t('col.engine'), t('col.address'), t('col.via'), t('col.status'), t('col.actions')]} {rows}>
	{#snippet cell(server, i)}
		{#if i === 0}<strong>{server.name}</strong>
		{:else if i === 1}<span class="inline-flex items-center gap-2"><BrandIcon name={server.engine} />{engines[server.engine].label}</span>
		{:else if i === 2}<code class="text-xs">{server.address}</code>
		{:else if i === 3}{server.agent}{server.ssh ? ` → ${server.ssh}` : ''}
		{:else if i === 4}<StatusBadge status={server.status} />
		{:else}<button type="button" class="btn" onclick={() => toast(t('toast.test', { name: server.name }))}>{t('action.test')}</button>{/if}
	{/snippet}
</DataTable>
