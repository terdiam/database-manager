<script lang="ts">
	import { onMount } from 'svelte';
	import { resolve } from '$app/paths';
	import { request, type DbServer, type Queued } from '#lib/api/http.ts';
	import BrandIcon from '#lib/components/BrandIcon.svelte';
	import ConfirmDialog from '#lib/components/ConfirmDialog.svelte';
	import DataTable from '#lib/components/DataTable.svelte';
	import JobDialog from '#lib/components/JobDialog.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import { toast } from '#lib/components/toast.svelte.ts';
	import { errorText } from '#lib/errors.ts';
	import { t } from '#lib/i18n/index.svelte.ts';
	import { live } from '#lib/events.ts';
	import { canOperate } from '#lib/session.svelte.ts';

	let servers = $state<DbServer[]>([]);
	let error = $state('');
	let loadError = $state('');
	let query = $state('');
	let removing = $state<DbServer>();
	let confirm: ConfirmDialog;
	let job: JobDialog;
	const rows = $derived(servers.filter((s) => `${s.name} ${s.host}`.toLowerCase().includes(query.toLowerCase())));

	const load = () =>
		request<DbServer[]>('GET', '/database-servers').then(
			(list) => {
				servers = list;
				loadError = '';
			},
			(e) => void (loadError = errorText(e))
		);
	onMount(() => {
		load();
		return live(['database_servers', 'agents'], load);
	});

	async function test(server: DbServer) {
		try {
			job.show((await request<Queued>('POST', `/database-servers/${server.id}/test`)).job_id);
		} catch (e) {
			error = errorText(e);
		}
	}

	async function remove() {
		try {
			await request('DELETE', `/database-servers/${removing!.id}`);
			toast(t('toast.deleted', { name: removing!.name }));
			error = '';
			await load();
		} catch (e) {
			error = errorText(e);
		}
	}
</script>

<PageHeader title={t('nav.databases')} description={t('databases.desc')}>
	{#snippet actions()}{#if canOperate()}<a class="btn btn-primary" href={resolve('databases/new')}>{t('databases.new')}</a>{/if}{/snippet}
</PageHeader>

<p class="mb-3 text-sm text-bad empty:hidden" role="alert">{error || loadError}</p>

<search class="mb-3 block max-w-xs">
	<input class="input" type="search" aria-label={t('action.search')} placeholder={t('action.search')} bind:value={query} />
</search>

<DataTable label={t('nav.databases')} columns={[t('col.name'), t('col.engine'), t('col.address'), 'User', t('col.via'), ...(canOperate() ? [t('col.actions')] : [])]} {rows}>
	{#snippet cell(server, i)}
		{#if i === 0}<strong>{server.name}</strong>
		{:else if i === 1}<span class="inline-flex items-center gap-2"><BrandIcon name={server.engine} />PostgreSQL</span>
		{:else if i === 2}<code class="text-xs">{server.host}:{server.port}</code>
		{:else if i === 3}{server.username}
		{:else if i === 4}{server.agent}
		{:else}<span class="flex flex-wrap justify-end gap-1">
			<button type="button" class="btn" onclick={() => test(server)}>{t('action.test')}</button>
			<button type="button" class="btn btn-danger" onclick={() => { removing = server; confirm.open(); }}>{t('action.delete')}</button>
		</span>{/if}
	{/snippet}
</DataTable>

<JobDialog bind:this={job} title={t('action.test')} />
<ConfirmDialog bind:this={confirm} title={t('action.delete')} message={t('confirm.delete', { name: removing?.name ?? '' })} confirmLabel={t('action.delete')} onconfirm={remove} />
