<script lang="ts">
	// New backup entry, or an existing one when `backup` is given.
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { request, type Backup, type DbServer, type Destination } from '#lib/api/http.ts';
	import { namePattern } from '#lib/api/index.ts';
	import FormField from './FormField.svelte';
	import PageHeader from './PageHeader.svelte';
	import RetentionPicker from './RetentionPicker.svelte';
	import SchedulePicker from './SchedulePicker.svelte';
	import Select from './Select.svelte';
	import { toast } from './toast.svelte.ts';
	import { errorText } from '#lib/errors.ts';
	import { t } from '#lib/i18n/index.svelte.ts';
	import { live } from '#lib/events.ts';
	import { fromCron, localZone, newRetention, newSchedule, retentionOf, toPlan } from '#lib/schedule.ts';

	let { backup }: { backup?: Backup } = $props();

	// The form is filled once: a live refresh must not overwrite what is being typed.
	// svelte-ignore state_referenced_locally
	const initial = backup;
	let servers = $state<DbServer[]>([]);
	let destinations = $state<Destination[]>([]);
	let error = $state('');
	let loadError = $state('');
	let form = $state({
		name: initial?.name ?? '',
		server_id: initial?.server_id ?? '',
		database_name: initial?.database_name ?? '',
		destination_id: initial?.destination_id ?? '',
		compression: initial?.compression ?? 'zstd',
		level: initial?.level ?? 3
	});
	let schedule = $state(initial ? fromCron(initial.schedule) : newSchedule('daily'));
	let timezone = $state(initial?.schedule ? initial.timezone : localZone());
	let retention = $state(initial ? retentionOf(initial) : newRetention());
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
		if (destinations.length && !usable.some((d) => d.id === form.destination_id)) form.destination_id = usable[0]?.id ?? '';
	});

	async function save(event: SubmitEvent) {
		event.preventDefault();
		try {
			const body = { ...form, level: Math.min(form.level, maxLevel), ...toPlan(schedule, timezone, retention) };
			if (initial) await request('PUT', `/backups/${initial.id}`, body);
			else await request('POST', '/backups', body);
			toast(t(initial ? 'toast.updated' : 'toast.created', { name: form.name }));
			await goto(resolve('backups'));
		} catch (e) {
			error = errorText(e);
		}
	}
</script>

<PageHeader title={initial ? `${t('action.edit')}: ${initial.name}` : t('backups.new')} />
<p class="mb-3 text-sm text-bad empty:hidden" role="alert">{error || loadError}</p>

<form class="card" onsubmit={save}>
	<FormField label={t('col.name')}>{#snippet children(a)}<input {...a} class="input" maxlength="64" required bind:value={form.name} />{/snippet}</FormField>
	<div class="grid gap-x-3 sm:grid-cols-2">
		<FormField label={t('nav.databases')}>{#snippet children(a)}<Select {...a} required bind:value={form.server_id} options={servers.map((s) => ({ value: s.id, label: s.name, brand: s.engine, description: `${s.engine === 'sqlite' ? `/${s.host}` : `${s.host}:${s.port}`} · ${s.agent}` }))} />{/snippet}</FormField>
		<FormField label={t(server?.engine === 'sqlite' ? 'field.fileName' : 'field.databaseName')} help={t(server?.engine === 'sqlite' ? 'help.fileName' : 'help.databaseName')}>{#snippet children(a)}<input {...a} class="input min-h-11" maxlength="63" pattern={namePattern(server?.engine)} required bind:value={form.database_name} />{/snippet}</FormField>
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
	{#if initial}<p class="mb-3 text-xs text-dim">{t('retention.fromNow')}</p>{/if}
	<p class="mb-3 rounded-lg bg-muted p-3 text-sm">{t('backups.phaseNote')}</p>
	<div class="flex justify-end gap-2">
		<a class="btn" href={resolve('backups')}>{t('action.cancel')}</a>
		<button type="submit" class="btn btn-primary">{t('action.save')}</button>
	</div>
</form>
