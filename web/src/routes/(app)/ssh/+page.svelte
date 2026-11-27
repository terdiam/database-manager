<script lang="ts">
	import { onMount } from 'svelte';
	import { request, type Agent, type Queued, type SshConfig } from '#lib/api/http.ts';
	import ConfirmDialog from '#lib/components/ConfirmDialog.svelte';
	import DataTable from '#lib/components/DataTable.svelte';
	import FormField from '#lib/components/FormField.svelte';
	import IconButton from '#lib/components/IconButton.svelte';
	import JobDialog from '#lib/components/JobDialog.svelte';
	import LoginFields from '#lib/components/LoginFields.svelte';
	import Modal from '#lib/components/Modal.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import Select from '#lib/components/Select.svelte';
	import { toast } from '#lib/components/toast.svelte.ts';
	import { errorText } from '#lib/errors.ts';
	import { live } from '#lib/events.ts';
	import { t } from '#lib/i18n/index.svelte.ts';
	import { sealFor } from '#lib/secrets.ts';
	import { canOperate } from '#lib/session.svelte.ts';

	const blank = () => ({ name: '', agent: '', host: '', port: 22, username: '', auth: 'password', password: '', private_key: '', passphrase: '', fingerprint: '' });
	let configs = $state<SshConfig[]>([]);
	let agents = $state<Agent[]>([]);
	let error = $state('');
	let loadError = $state('');
	let formError = $state('');
	let form = $state(blank());
	let editing = $state<SshConfig>();
	let reconnect = $state(true);
	let removing = $state<SshConfig>();
	let modal: Modal;
	let confirm: ConfirmDialog;
	let job: JobDialog;

	const load = () =>
		request<SshConfig[]>('GET', '/ssh-configs').then(
			(list) => {
				configs = list;
				loadError = '';
			},
			(e) => void (loadError = errorText(e))
		);
	onMount(() => {
		load();
		return live(['ssh_configs', 'database_servers'], load);
	});

	async function open(config?: SshConfig) {
		editing = config;
		reconnect = !config;
		form = config ? { ...blank(), name: config.name, agent: config.agent_id, host: config.host, port: config.port, username: config.username, auth: config.auth } : blank();
		formError = '';
		modal.open();
		try {
			agents = (await request<Agent[]>('GET', '/agents')).filter((a) => a.key);
			form.agent ||= agents[0]?.id ?? '';
		} catch (e) {
			formError = errorText(e);
		}
	}

	async function save(close: () => void) {
		const id = editing?.id ?? crypto.randomUUID();
		const { name, agent, auth, port, fingerprint } = form;
		const host = form.host.trim();
		const username = form.username.trim();
		const key = auth === 'key';
		try {
			const connection = reconnect
				? {
						host, port, username, auth,
						secret: await sealFor(agents.find((a) => a.id === agent), 'ssh', id, { host, port, username, password: key ? '' : form.password, private_key: key ? form.private_key : '', passphrase: key ? form.passphrase : '', fingerprint: fingerprint.trim() })
					}
				: undefined;
			if (editing) await request('PUT', `/ssh-configs/${id}`, { name, connection });
			else await request('POST', '/ssh-configs', { id, name, agent_id: agent, ...connection });
			close();
			form = blank();
			toast(t(editing ? 'toast.updated' : 'toast.created', { name }));
			await load();
		} catch (e) {
			formError = errorText(e);
		}
	}

	async function test(config: SshConfig) {
		try {
			job.show((await request<Queued>('POST', `/ssh-configs/${config.id}/test`)).job_id);
		} catch (e) {
			error = errorText(e);
		}
	}

	async function remove() {
		try {
			await request('DELETE', `/ssh-configs/${removing!.id}`);
			toast(t('toast.deleted', { name: removing!.name }));
			error = '';
			await load();
		} catch (e) {
			error = errorText(e);
		}
	}
</script>

<PageHeader title={t('nav.ssh')} description={t('ssh.desc')}>
	{#snippet actions()}{#if canOperate()}<button type="button" class="btn btn-primary" onclick={() => open()}>{t('ssh.new')}</button>{/if}{/snippet}
</PageHeader>

<p class="mb-3 text-sm text-bad empty:hidden" role="alert">{error || loadError}</p>

<DataTable label={t('nav.ssh')} columns={[t('col.name'), t('col.address'), t('col.auth'), t('col.usedBy'), t('col.via'), ...(canOperate() ? [t('col.actions')] : [])]} rows={configs}>
	{#snippet cell(c, i)}
		{#if i === 0}<strong>{c.name}</strong>
		{:else if i === 1}<code class="text-xs break-all">{c.username}@{c.host}:{c.port}</code>
		{:else if i === 2}{t(`auth.${c.auth}`)}
		{:else if i === 3}{t('ssh.used', { n: c.used })}
		{:else if i === 4}{c.agent}
		{:else}<span class="flex flex-wrap justify-end gap-1">
			<IconButton icon="plug" label={t('action.test')} onclick={() => test(c)} />
			<IconButton icon="edit" label={t('action.edit')} onclick={() => open(c)} />
			<IconButton icon="trash" label={t('action.delete')} onclick={() => { removing = c; confirm.open(); }} danger />
		</span>{/if}
	{/snippet}
</DataTable>

<Modal bind:this={modal} title={editing ? `${t('action.edit')}: ${editing.name}` : t('ssh.new')}>
	{#snippet children(close)}
		<form onsubmit={(e) => { e.preventDefault(); save(close); }}>
			<FormField label={t('col.name')}>{#snippet children(a)}<input {...a} class="input" maxlength="64" required bind:value={form.name} />{/snippet}</FormField>
			{#if editing}
				<label class="mb-1 flex items-center gap-2 text-sm font-medium"><input type="checkbox" bind:checked={reconnect} />{t('field.changeConnection')}</label>
				<p class="mb-3 text-xs text-dim">{t('help.changeConnection', { agent: editing.agent })}</p>
			{:else}
				<FormField label={t('field.agent')} help={t('help.agent')}>{#snippet children(a)}<Select {...a} required bind:value={form.agent} options={agents.map((x) => ({ value: x.id, label: x.name, icon: 'server' as const }))} />{/snippet}</FormField>
			{/if}
			{#if reconnect}<LoginFields bind:form />{/if}
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
