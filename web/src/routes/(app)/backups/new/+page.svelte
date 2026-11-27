<script lang="ts">
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { request, type DbServer, type Destination } from '#lib/api/http.ts';
	import FormField from '#lib/components/FormField.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import RetentionPicker from '#lib/components/RetentionPicker.svelte';
	import SchedulePicker from '#lib/components/SchedulePicker.svelte';
	import Select from '#lib/components/Select.svelte';
	import { toast } from '#lib/components/toast.svelte.ts';
	import { errorText } from '#lib/errors.ts';
	import { t } from '#lib/i18n/index.svelte.ts';
	import { live } from '#lib/events.ts';
	import { localZone, newRetention, newSchedule, toPlan } from '#lib/schedule.ts';

	let servers = $state<DbServer[]>([]);
	let destinations = $state<Destination[]>([]);
	let error = $state('');
	let loadError = $state('');
	let form = $state({ name: '', server_id: '', database_name: '', destination_id: '', compression: 'zstd', level: 3 });
	let schedule = $state(newSchedule('daily'));
	let timezone = $state(localZone());
	let retention = $state(newRetention());
	const server = $derived(servers.find((s) => s.id === form.server_id));
	// The agent that dumps is the one that uploads.
	const usable = $derived(destinations.filter((d) => d.agent_id === server?.agent_id));
	const maxLevel = $derived(form.compression === 'zstd' ? 19 : 9);

	async function load() {
		try {
			[servers, destinations] = await Promise.all([request<DbServer[]>('GET', '/database-servers'), request<Destination[]>('GET', '/destinations')]);
			if (!servers.some((s) => s.id === form.server_id)) form.server_id = servers[0]?.id ?? '';
			loadError = '';
		} catch (e) {
			loadError = errorText(e);
		}
	}
	onMount(() => {
		load();
		return live(['database_servers', 'destinations'], load);
	});

	$effect(() => {
		if (!usable.some((d) => d.id === form.destination_id)) form.destination_id = usable[0]?.id ?? '';
	});

	async function save(event: SubmitEvent) {
		event.preventDefault();
		try {
			await request('POST', '/backups', { ...form, level: Math.min(form.level, maxLevel), ...toPlan(schedule, timezone, retention) });
			toast(t('toast.created', { name: form.name }));
			await goto(resolve('backups'));
		} catch (e) {
			error = errorText(e);
		}
	}
</script>

<PageHeader title={t('backups.new')} />
<p class="mb-3 text-sm text-bad empty:hidden" role="alert">{error || loadError}</p>

<form class="card" onsubmit={save}>
	<FormField label={t('col.name')}>{#snippet children(a)}<input {...a} class="input" maxlength="64" required bind:value={form.name} />{/snippet}</FormField>
	<div class="grid gap-x-3 sm:grid-cols-2">
		<FormField label={t('nav.databases')}>{#snippet children(a)}<Select {...a} required bind:value={form.server_id} options={servers.map((s) => ({ value: s.id, label: s.name, brand: s.engine, description: `${s.host}:${s.port} · ${s.agent}` }))} />{/snippet}</FormField>
		<FormField label={t('field.databaseName')} help={t('help.databaseName')}>{#snippet children(a)}<input {...a} class="input min-h-11" maxlength="63" pattern={'[A-Za-z0-9_][A-Za-z0-9_\\-]*'} required bind:value={form.database_name} />{/snippet}</FormField>
	</div>
	<FormField label={t('col.destination')} help={t('help.destinationAgent')}>{#snippet children(a)}<Select {...a} required bind:value={form.destination_id} options={usable.map((d) => ({ value: d.id, label: d.name, brand: 'S3 / MinIO', description: `s3://${d.bucket}/${d.prefix}` }))} />{/snippet}</FormField>
	{#if server && !usable.length}<p class="mb-3 text-sm text-warn" role="alert">{t('backups.noDestination', { agent: server.agent })} <a class="underline" href={resolve('destinations')}>{t('nav.destinations')}</a></p>{/if}
	<div class="grid gap-x-3 sm:grid-cols-2">
		<FormField label={t('wizard.compression')} help={t('help.compression')}>{#snippet children(a)}<Select {...a} bind:value={form.compression} options={(['zstd', 'gzip', 'lz4', 'xz', 'bzip2', 'none'] as const).map((c) => ({ value: c, label: c, description: t(`compression.${c}`) }))} />{/snippet}</FormField>
		{#if form.compression !== 'none' && form.compression !== 'lz4'}
			<FormField label={t('field.level')}>{#snippet children(a)}<input {...a} class="input min-h-11" type="number" min="1" max={maxLevel} required bind:value={form.level} />{/snippet}</FormField>
		{/if}
	</div>
	<SchedulePicker legend={t('col.schedule')} bind:value={schedule} bind:timezone />
	<RetentionPicker bind:value={retention} />
	<p class="mb-3 rounded-lg bg-muted p-3 text-sm">{t('backups.phaseNote')}</p>
	<div class="flex justify-end gap-2">
		<a class="btn" href={resolve('backups')}>{t('action.cancel')}</a>
		<button type="submit" class="btn btn-primary">{t('action.save')}</button>
	</div>
</form>
