<script lang="ts">
	import IconButton from '#lib/components/IconButton.svelte';
	import { onMount } from 'svelte';
	import { request, type Agent, type Destination, type Queued } from '#lib/api/http.ts';
	import BrandIcon from '#lib/components/BrandIcon.svelte';
	import ConfirmDialog from '#lib/components/ConfirmDialog.svelte';
	import DataTable from '#lib/components/DataTable.svelte';
	import DestinationFields from '#lib/components/DestinationFields.svelte';
	import FormField from '#lib/components/FormField.svelte';
	import JobDialog from '#lib/components/JobDialog.svelte';
	import Modal from '#lib/components/Modal.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import Select from '#lib/components/Select.svelte';
	import { toast } from '#lib/components/toast.svelte.ts';
	import { errorText } from '#lib/errors.ts';
	import { t } from '#lib/i18n/index.svelte.ts';
	import { live } from '#lib/events.ts';
	import { sealFor } from '#lib/secrets.ts';
	import { blankStorage, connectionOf, formOf, kinds, type Kind } from '#lib/storage.ts';
	import { canOperate } from '#lib/session.svelte.ts';

	let destinations = $state<Destination[]>([]);
	let agents = $state<Agent[]>([]);
	let error = $state('');
	let loadError = $state('');
	let formError = $state('');
	let form = $state(blankStorage());
	let removing = $state<Destination>();
	// Set while the dialog edits an existing destination.
	let editing = $state<Destination>();
	let reconnect = $state(true);
	let modal: Modal;
	let confirm: ConfirmDialog;
	let job: JobDialog;

	const load = () =>
		request<Destination[]>('GET', '/destinations').then(
			(list) => {
				destinations = list;
				loadError = '';
			},
			(e) => void (loadError = errorText(e))
		);
	onMount(() => {
		load();
		return live(['destinations'], load);
	});

	async function open(destination?: Destination) {
		editing = destination;
		reconnect = !destination;
		form = destination ? formOf(destination) : blankStorage();
		formError = '';
		modal.open();
		try {
			agents = (await request<Agent[]>('GET', '/agents')).filter((a) => a.key);
			form.agent ||= agents[0]?.id ?? '';
		} catch (e) {
			formError = errorText(e);
		}
	}

	async function create(event: SubmitEvent & { currentTarget: HTMLFormElement }, close: () => void) {
		event.preventDefault();
		const element = event.currentTarget;
		const id = editing?.id ?? crypto.randomUUID();
		const { name, agent, kind } = form;
		try {
			let connection;
			if (reconnect) {
				const { record, secret } = connectionOf(form);
				connection = { ...record, secret: await sealFor(agents.find((a) => a.id === agent), 'destination', id, secret) };
			}
			if (editing) await request('PUT', `/destinations/${id}`, { name, connection });
			else await request('POST', '/destinations', { id, name, kind, agent_id: agent, ...connection });
			close();
			element.reset();
			toast(t(editing ? 'toast.updated' : 'toast.created', { name }));
			await load();
		} catch (e) {
			formError = errorText(e);
		}
	}

	async function test(destination: Destination) {
		try {
			job.show((await request<Queued>('POST', `/destinations/${destination.id}/test`)).job_id);
		} catch (e) {
			error = errorText(e);
		}
	}

	async function remove() {
		try {
			await request('DELETE', `/destinations/${removing!.id}`);
			toast(t('toast.deleted', { name: removing!.name }));
			error = '';
			await load();
		} catch (e) {
			error = errorText(e);
		}
	}
</script>

<PageHeader title={t('nav.destinations')} description={t('destinations.desc')}>
	{#snippet actions()}{#if canOperate()}<button type="button" class="btn btn-primary" onclick={() => open()}>{t('destinations.new')}</button>{/if}{/snippet}
</PageHeader>

<p class="mb-3 text-sm text-bad empty:hidden" role="alert">{error || loadError}</p>

<DataTable label={t('nav.destinations')} columns={[t('col.name'), t('col.type'), t('col.target'), t('col.via'), ...(canOperate() ? [t('col.actions')] : [])]} rows={destinations}>
	{#snippet cell(d, i)}
		{#if i === 0}<strong>{d.name}</strong>
		{:else if i === 1}<span class="inline-flex items-center gap-2"><BrandIcon name={kinds[d.kind].brand} />{kinds[d.kind].label}</span>
		{:else if i === 2}<code class="text-xs break-all">{d.kind === 's3' ? `${d.endpoint || 'AWS'} · s3://${d.bucket}/${d.prefix}` : `${d.endpoint}/${d.bucket ? `${d.bucket}/` : ''}${d.prefix}`}</code>
		{:else if i === 3}{d.agent}
		{:else}<span class="flex flex-wrap justify-end gap-1">
			<IconButton icon="plug" label={t('action.test')} onclick={() => test(d)} />
			<IconButton icon="edit" label={t('action.edit')} onclick={() => open(d)} />
			<IconButton icon="trash" label={t('action.delete')} onclick={() => { removing = d; confirm.open(); }} danger />
		</span>{/if}
	{/snippet}
</DataTable>

<Modal bind:this={modal} title={editing ? `${t('action.edit')}: ${editing.name}` : t('destinations.new')}>
	{#snippet children(close)}
		<form onsubmit={(e) => create(e, close)}>
			<FormField label={t('col.name')}>{#snippet children(a)}<input {...a} class="input" maxlength="64" required bind:value={form.name} />{/snippet}</FormField>
			{#if editing}
				<label class="mb-1 flex items-center gap-2 text-sm font-medium"><input type="checkbox" bind:checked={reconnect} />{t('field.changeConnection')}</label>
				<p class="mb-3 text-xs text-dim">{t('help.changeConnection', { agent: editing.agent })}</p>
			{:else}
				<FormField label={t('col.type')}>{#snippet children(a)}<Select {...a} bind:value={() => form.kind, (kind) => (form = { ...blankStorage(kind as Kind), name: form.name, agent: form.agent })} options={Object.entries(kinds).map(([value, k]) => ({ value, label: k.label, brand: k.brand }))} />{/snippet}</FormField>
				<FormField label={t('field.agent')} help={t('help.destinationAgent')}>{#snippet children(a)}<Select {...a} required bind:value={form.agent} options={agents.map((x) => ({ value: x.id, label: x.name, icon: 'server' as const }))} />{/snippet}</FormField>
			{/if}
			{#if reconnect}<DestinationFields bind:form />{/if}
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
