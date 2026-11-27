<script lang="ts">
	import { onMount } from 'svelte';
	import { request, type Agent, type Destination, type Queued } from '#lib/api/http.ts';
	import BrandIcon from '#lib/components/BrandIcon.svelte';
	import ConfirmDialog from '#lib/components/ConfirmDialog.svelte';
	import DataTable from '#lib/components/DataTable.svelte';
	import FormField from '#lib/components/FormField.svelte';
	import JobDialog from '#lib/components/JobDialog.svelte';
	import Modal from '#lib/components/Modal.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import PasswordInput from '#lib/components/PasswordInput.svelte';
	import Select from '#lib/components/Select.svelte';
	import { toast } from '#lib/components/toast.svelte.ts';
	import { errorText } from '#lib/errors.ts';
	import { t } from '#lib/i18n/index.svelte.ts';
	import { live } from '#lib/events.ts';
	import { sealFor } from '#lib/secrets.ts';
	import { canOperate } from '#lib/session.svelte.ts';

	const blank = () => ({ name: '', agent: '', endpoint: '', region: '', bucket: '', prefix: '', access_key: '', secret_key: '', virtual_host: false, public_endpoint: '' });
	let destinations = $state<Destination[]>([]);
	let agents = $state<Agent[]>([]);
	let error = $state('');
	let loadError = $state('');
	let formError = $state('');
	let form = $state(blank());
	let removing = $state<Destination>();
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

	async function open() {
		form = blank();
		formError = '';
		modal.open();
		try {
			agents = (await request<Agent[]>('GET', '/agents')).filter((a) => a.key);
			form.agent = agents[0]?.id ?? '';
		} catch (e) {
			formError = errorText(e);
		}
	}

	async function create(event: SubmitEvent & { currentTarget: HTMLFormElement }, close: () => void) {
		event.preventDefault();
		const element = event.currentTarget;
		const id = crypto.randomUUID();
		const { name, agent, endpoint, region, bucket, prefix, access_key, secret_key, virtual_host, public_endpoint } = form;
		try {
			const secret = await sealFor(agents.find((a) => a.id === agent), 'destination', id, { endpoint: endpoint.trim(), region: region.trim(), bucket: bucket.trim(), prefix: prefix.trim(), access_key, secret_key, virtual_host, public_endpoint: public_endpoint.trim() });
			await request('POST', '/destinations', { id, name, agent_id: agent, endpoint, bucket, prefix, secret });
			close();
			element.reset();
			toast(t('toast.created', { name }));
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
	{#snippet actions()}{#if canOperate()}<button type="button" class="btn btn-primary" onclick={open}>{t('destinations.new')}</button>{/if}{/snippet}
</PageHeader>

<p class="mb-3 text-sm text-bad empty:hidden" role="alert">{error || loadError}</p>

<DataTable label={t('nav.destinations')} columns={[t('col.name'), t('col.type'), t('col.target'), t('col.via'), ...(canOperate() ? [t('col.actions')] : [])]} rows={destinations}>
	{#snippet cell(d, i)}
		{#if i === 0}<strong>{d.name}</strong>
		{:else if i === 1}<span class="inline-flex items-center gap-2"><BrandIcon name="S3 / MinIO" />S3 / MinIO</span>
		{:else if i === 2}<code class="text-xs break-all">{d.endpoint || 'AWS'} · s3://{d.bucket}/{d.prefix}</code>
		{:else if i === 3}{d.agent}
		{:else}<span class="flex flex-wrap justify-end gap-1">
			<button type="button" class="btn" onclick={() => test(d)}>{t('action.test')}</button>
			<button type="button" class="btn btn-danger" onclick={() => { removing = d; confirm.open(); }}>{t('action.delete')}</button>
		</span>{/if}
	{/snippet}
</DataTable>

<Modal bind:this={modal} title={t('destinations.new')}>
	{#snippet children(close)}
		<form onsubmit={(e) => create(e, close)}>
			<p class="mb-3 text-sm text-dim">{t('destinations.s3Only')}</p>
			<FormField label={t('col.name')}>{#snippet children(a)}<input {...a} class="input" maxlength="64" required bind:value={form.name} />{/snippet}</FormField>
			<FormField label={t('field.agent')} help={t('help.destinationAgent')}>{#snippet children(a)}<Select {...a} required bind:value={form.agent} options={agents.map((x) => ({ value: x.id, label: x.name, icon: 'server' as const }))} />{/snippet}</FormField>
			<FormField label="Endpoint" help={t('help.endpoint')}>{#snippet children(a)}<input {...a} class="input" type="url" maxlength="255" placeholder="http://minio:9000" bind:value={form.endpoint} />{/snippet}</FormField>
			<FormField label={t('field.publicEndpoint')} help={t('help.publicEndpoint')}>{#snippet children(a)}<input {...a} class="input" type="url" maxlength="255" bind:value={form.public_endpoint} />{/snippet}</FormField>
			<div class="grid gap-x-3 sm:grid-cols-2">
				<FormField label="Bucket">{#snippet children(a)}<input {...a} class="input" maxlength="255" required bind:value={form.bucket} />{/snippet}</FormField>
				<FormField label="Region">{#snippet children(a)}<input {...a} class="input" maxlength="64" placeholder="us-east-1" bind:value={form.region} />{/snippet}</FormField>
			</div>
			<FormField label={t('field.prefix')}>{#snippet children(a)}<input {...a} class="input" maxlength="255" placeholder="backups/" bind:value={form.prefix} />{/snippet}</FormField>
			<FormField label="Access key">{#snippet children(a)}<input {...a} class="input" autocomplete="off" required bind:value={form.access_key} />{/snippet}</FormField>
			<FormField label="Secret key" help={t('help.sealed')}>{#snippet children(a)}<PasswordInput {...a} autocomplete="new-password" required bind:value={form.secret_key} />{/snippet}</FormField>
			<label class="mb-3 flex items-center gap-2 text-sm"><input type="checkbox" bind:checked={form.virtual_host} />{t('field.virtualHost')}</label>
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
