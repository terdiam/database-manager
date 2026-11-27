<script lang="ts">
	import { onMount } from 'svelte';
	import { e2eeInfo, request, type Agent } from '#lib/api/http.ts';
	import ConfirmDialog from '#lib/components/ConfirmDialog.svelte';
	import DataTable from '#lib/components/DataTable.svelte';
	import EmptyState from '#lib/components/EmptyState.svelte';
	import FormField from '#lib/components/FormField.svelte';
	import Modal from '#lib/components/Modal.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import StatusBadge from '#lib/components/StatusBadge.svelte';
	import { toast } from '#lib/components/toast.svelte.ts';
	import { errorText } from '#lib/errors.ts';
	import { dateTime } from '#lib/format.ts';
	import { t } from '#lib/i18n/index.svelte.ts';
	import { live } from '#lib/events.ts';
	import { isAdmin } from '#lib/session.svelte.ts';

	let agents = $state<Agent[]>();
	let loadError = $state('');
	let actionError = $state('');
	let name = $state('');
	let token = $state('');
	let formError = $state('');
	let serverFingerprint = $state('');
	let removing = $state<Agent>();
	let modal: Modal;
	let confirm: ConfirmDialog;

	const command = $derived(
		`DBM_SERVER_URL=${location.origin} \\\nDBM_ENROLL_TOKEN=${token} \\\nDBM_SERVER_FINGERPRINT=${serverFingerprint} \\\nDBM_STATE_FILE=./agent-state \\\ndbm-agent`
	);

	async function load() {
		try {
			agents = await request<Agent[]>('GET', '/agents');
			loadError = '';
		} catch (e) {
			loadError = errorText(e);
		}
	}

	onMount(() => {
		load();
		return live(['agents'], load, { every: 30_000 });
	});

	function open() {
		name = token = formError = '';
		modal.open();
	}

	async function createToken(event: SubmitEvent) {
		event.preventDefault();
		try {
			serverFingerprint = (await e2eeInfo()).fingerprint;
			token = (await request<{ token: string }>('POST', '/agents/tokens', { name })).token;
			formError = '';
		} catch (e) {
			formError = errorText(e);
		}
	}

	async function copy() {
		try {
			await navigator.clipboard.writeText(command);
			toast(t('toast.copied'));
		} catch {
			formError = t('toast.copyFailed');
		}
	}

	async function remove() {
		try {
			await request('DELETE', `/agents/${removing!.id}`);
			toast(t('toast.deleted', { name: removing!.name }));
		} catch (e) {
			actionError = errorText(e);
			return;
		}
		actionError = '';
		await load();
	}
</script>

<PageHeader title={t('nav.agents')} description={t('agents.desc')}>
	{#snippet actions()}{#if isAdmin()}<button type="button" class="btn btn-primary" onclick={open}>{t('agents.new')}</button>{/if}{/snippet}
</PageHeader>

<p class="mb-3 text-sm text-bad empty:hidden" role="alert">{actionError || loadError}</p>

{#if agents?.length === 0}
	<EmptyState title={t('agents.none')} hint={t('agents.noneHint')} />
{:else if agents}
	<DataTable label={t('nav.agents')} columns={[t('col.name'), t('col.status'), t('col.version'), t('col.seen'), t('agents.fingerprint'), ...(isAdmin() ? [t('col.actions')] : [])]} rows={agents}>
		{#snippet cell(agent, i)}
			{#if i === 0}<strong>{agent.name}</strong>
			{:else if i === 1}<StatusBadge status={agent.online ? 'online' : 'offline'} />
			{:else if i === 2}{agent.version}
			{:else if i === 3}{dateTime(agent.last_seen_at)}
			{:else if i === 4}<code class="text-xs">{agent.fingerprint ?? t('agents.noKey')}</code>
			{:else}<button type="button" class="btn btn-danger" onclick={() => { removing = agent; confirm.open(); }}>{t('action.delete')}</button>{/if}
		{/snippet}
	</DataTable>
{/if}

<Modal bind:this={modal} title={t('agents.new')}>
	{#snippet children(close)}
		{#if token}
			<p class="mb-2 text-sm">{t('agents.install')}</p>
			<pre class="overflow-x-auto rounded-lg bg-muted p-3 text-xs">{command}</pre>
			<p class="mt-2 text-sm text-bad empty:hidden" role="alert">{formError}</p>
			<div class="mt-4 flex justify-end gap-2">
				<button type="button" class="btn" onclick={() => { close(); load(); }}>{t('action.close')}</button>
				<button type="button" class="btn btn-primary" onclick={copy}>{t('action.copy')}</button>
			</div>
		{:else}
			<form onsubmit={createToken}>
				<FormField label={t('agents.name')} help={t('agents.nameHelp')}>{#snippet children(a)}<input {...a} class="input" maxlength="64" autocomplete="off" required bind:value={name} />{/snippet}</FormField>
				<p class="text-sm text-bad empty:hidden" role="alert">{formError}</p>
				<div class="mt-4 flex justify-end gap-2">
					<button type="button" class="btn" onclick={close}>{t('action.cancel')}</button>
					<button type="submit" class="btn btn-primary">{t('agents.createToken')}</button>
				</div>
			</form>
		{/if}
	{/snippet}
</Modal>

<ConfirmDialog bind:this={confirm} title={t('action.delete')} message={t('agents.confirmDelete', { name: removing?.name ?? '' })} confirmLabel={t('action.delete')} onconfirm={remove} />
