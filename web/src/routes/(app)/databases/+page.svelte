<script lang="ts">
	import IconButton from '#lib/components/IconButton.svelte';
	import { onMount } from 'svelte';
	import { resolve } from '$app/paths';
	import { request, type Agent, type DbServer, type Queued, type SshConfig } from '#lib/api/http.ts';
	import { connectionOf, engines } from '#lib/api/index.ts';
	import BrandIcon from '#lib/components/BrandIcon.svelte';
	import ConfirmDialog from '#lib/components/ConfirmDialog.svelte';
	import ConnectionFields from '#lib/components/ConnectionFields.svelte';
	import DataTable from '#lib/components/DataTable.svelte';
	import FormField from '#lib/components/FormField.svelte';
	import Modal from '#lib/components/Modal.svelte';
	import JobDialog from '#lib/components/JobDialog.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import Select from '#lib/components/Select.svelte';
	import { toast } from '#lib/components/toast.svelte.ts';
	import { errorText } from '#lib/errors.ts';
	import { t } from '#lib/i18n/index.svelte.ts';
	import { live } from '#lib/events.ts';
	import { sealFor } from '#lib/secrets.ts';
	import { canOperate } from '#lib/session.svelte.ts';

	let servers = $state<DbServer[]>([]);
	let error = $state('');
	let loadError = $state('');
	let query = $state('');
	let removing = $state<DbServer>();
	let confirm: ConfirmDialog;
	let job: JobDialog;
	let editor: Modal;
	let editing = $state<DbServer>();
	let agent = $state<Agent>();
	let tunnels = $state<SshConfig[]>([]);
	let form = $state({ name: '', reconnect: false, host: '', port: 5432, username: '', password: '', ssl_mode: '', ssh_config_id: '' });
	let formError = $state('');

	async function edit(server: DbServer) {
		editing = server;
		form = { name: server.name, reconnect: false, host: server.host, port: server.port, username: server.username, password: '', ssl_mode: server.ssl_mode ?? '', ssh_config_id: server.ssh_config_id ?? '' };
		formError = '';
		agent = undefined;
		editor.open();
		try {
			const [agents, configs] = await Promise.all([request<Agent[]>('GET', '/agents'), request<SshConfig[]>('GET', '/ssh-configs')]);
			agent = agents.find((a) => a.id === server.agent_id);
			tunnels = configs;
		} catch (e) {
			formError = errorText(e);
		}
	}

	async function save(close: () => void) {
		const { name, reconnect } = form;
		const { id, engine } = editing!;
		try {
			// Sealed to the same record id: host, user and password travel together.
			let connection;
			if (reconnect) {
				const values = connectionOf(engine, form);
				const secret = await sealFor(agent, 'database', id, { engine, ...values });
				const { host, port, username, ssl_mode } = values;
				connection = { host, port, username, ssl_mode, secret };
			}
			await request('PUT', `/database-servers/${id}`, { name, connection, ssh_config_id: form.ssh_config_id || null });
			close();
			form.password = '';
			toast(t('toast.updated', { name }));
			await load();
		} catch (e) {
			formError = errorText(e);
		}
	}
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
		{:else if i === 1}<span class="inline-flex items-center gap-2"><BrandIcon name={server.engine} />{engines[server.engine].label}</span>
		{:else if i === 2}<code class="text-xs">{server.engine === 'sqlite' ? `/${server.host}` : `${server.host}:${server.port}`}</code>
		{:else if i === 3}{server.username}
		{:else if i === 4}{server.agent}{#if server.ssh}<span class="block text-xs text-dim">SSH: {server.ssh}</span>{/if}
		{:else}<span class="flex flex-wrap justify-end gap-1">
			<IconButton icon="plug" label={t('action.test')} onclick={() => test(server)} />
			<IconButton icon="edit" label={t('action.edit')} onclick={() => edit(server)} />
			<IconButton icon="trash" label={t('action.delete')} onclick={() => { removing = server; confirm.open(); }} danger />
		</span>{/if}
	{/snippet}
</DataTable>

<Modal bind:this={editor} title="{t('action.edit')}: {editing?.name ?? ''}">
	{#snippet children(close)}
		<form onsubmit={(e) => { e.preventDefault(); save(close); }}>
			<FormField label={t('col.name')}>{#snippet children(a)}<input {...a} class="input" maxlength="64" required bind:value={form.name} />{/snippet}</FormField>
			<label class="mb-1 flex items-center gap-2 text-sm font-medium"><input type="checkbox" bind:checked={form.reconnect} />{t('field.changeConnection')}</label>
			<p class="mb-3 text-xs text-dim">{t('help.changeConnection', { agent: editing?.agent ?? '' })}</p>
			{#if form.reconnect && editing}<ConnectionFields engine={editing.engine} bind:form />{/if}
			{#if editing && editing.engine !== 'sqlite'}<FormField label="SSH tunnel" help={t('help.tunnel')}>{#snippet children(a)}<Select {...a} bind:value={form.ssh_config_id} options={[{ value: '', label: t('field.noTunnel') }, ...tunnels.filter((c) => c.agent_id === editing?.agent_id).map((c) => ({ value: c.id, label: c.name, icon: 'key' as const, description: `${c.username}@${c.host}` }))]} />{/snippet}</FormField>{/if}
			<p class="text-sm text-bad empty:hidden" role="alert">{formError}</p>
			<div class="mt-4 flex justify-end gap-2">
				<button type="button" class="btn" onclick={close}>{t('action.cancel')}</button>
				<button type="submit" class="btn btn-primary">{t('action.save')}</button>
			</div>
		</form>
	{/snippet}
</Modal>

<JobDialog bind:this={job} title={t('action.test')} />
<ConfirmDialog bind:this={confirm} title={t('action.delete')} message={t('confirm.delete', { name: removing?.name ?? '' })} confirmLabel={t('action.delete')} onconfirm={remove} />
