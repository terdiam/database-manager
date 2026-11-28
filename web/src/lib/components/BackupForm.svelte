<script lang="ts">
	// New backup entry, or an existing one when `backup` is given.
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { request, type Backup, type DbServer, type Destination } from '#lib/api/http.ts';
	import { canFollow, namePattern, targetField } from '#lib/api/index.ts';
	import { kinds } from '#lib/storage.ts';
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
	// An incremental entry follows a full one and takes everything but its name and schedule.
	let kind = $state(initial?.base_id ? 'incremental' : 'full');
	let baseId = $state(initial?.base_id ?? '');
	let backups = $state<Backup[]>([]);
	const engineOf = (b: Backup) => servers.find((s) => s.id === b.server_id)?.engine;
	const bases = $derived(
		backups.filter((b) => {
			const engine = engineOf(b);
			return !b.base_id && b.retention_unit !== 'gfs' && !!engine && canFollow(engine, b.database_name);
		})
	);
	const base = $derived(bases.find((b) => b.id === baseId));
	const baseEngine = $derived(base && engineOf(base));

	$effect(() => {
		if (kind === 'incremental' && bases.length && !base) baseId = bases[0].id;
	});

	const usable = $derived(destinations.filter((d) => d.agent_id === server?.agent_id));
	const target = $derived(targetField(server?.engine));
	const maxLevel = $derived(form.compression === 'zstd' ? 19 : 9);

	async function load() {
		try {
			[servers, destinations, backups] = await Promise.all([request<DbServer[]>('GET', '/database-servers'), request<Destination[]>('GET', '/destinations'), request<Backup[]>('GET', '/backups')]);
			if (!servers.some((s) => s.id === form.server_id)) form.server_id = servers[0]?.id ?? '';
			loadError = '';
		} catch (e) {
			loadError = errorText(e);
		}
	}
	onMount(() => {
		load();
		return live(['database_servers', 'destinations', 'backup_configs'], load);
	});

	// Redis has no database names: the whole instance is the only choice.
	$effect(() => {
		if (server?.engine === 'redis') form.database_name = 'all';
		else if (server && !initial && form.database_name === 'all') form.database_name = '';
	});

	$effect(() => {
		if (destinations.length && !usable.some((d) => d.id === form.destination_id)) form.destination_id = usable[0]?.id ?? '';
	});

	async function save(event: SubmitEvent) {
		event.preventDefault();
		try {
			const plan = toPlan(schedule, timezone, retention);
			const body =
				kind === 'incremental'
					? { name: form.name, base_id: baseId, schedule: plan.schedule, timezone: plan.timezone }
					: { ...form, level: Math.min(form.level, maxLevel), ...plan };
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
	<!-- The kind of an entry cannot change later. -->
	{#if !initial}
		<FormField label={t('backups.type')} help={t(`backups.type.${kind === 'full' ? 'full' : 'incremental'}`)}>{#snippet children(a)}<Select {...a} bind:value={kind} options={[{ value: 'full', label: t('type.full') }, { value: 'incremental', label: t('type.incremental') }]} />{/snippet}</FormField>
	{/if}
	{#if kind === 'incremental'}
		{#if bases.length}
			<FormField label={t('backups.base')} help={t('backups.baseHelp')}>{#snippet children(a)}<Select {...a} required bind:value={baseId} options={bases.map((b) => ({ value: b.id, label: b.name, brand: engineOf(b), description: `${b.server} / ${b.database_name}` }))} />{/snippet}</FormField>
			<p class="mb-3 text-sm text-dim">{t('backups.inherits')}</p>
			{#if baseEngine === 'mysql' || baseEngine === 'mariadb' || baseEngine === 'mongodb'}<p class="mb-3 rounded-lg bg-muted p-3 text-sm">{t(`requirement.${baseEngine}`)}</p>{/if}
		{:else}
			<p class="mb-3 text-sm text-warn" role="alert">{t('backups.noBase')}</p>
		{/if}
		<SchedulePicker legend={t('col.schedule')} bind:value={schedule} bind:timezone />
	{:else}
	<div class="grid gap-x-3 sm:grid-cols-2">
		<FormField label={t('nav.databases')}>{#snippet children(a)}<Select {...a} required bind:value={form.server_id} options={servers.map((s) => ({ value: s.id, label: s.name, brand: s.engine, description: `${s.engine === 'sqlite' ? `/${s.host}` : `${s.host}:${s.port}`} · ${s.agent}` }))} />{/snippet}</FormField>
		<FormField label={t(target.label)} help={t(target.help)}>{#snippet children(a)}<input {...a} class="input min-h-11" maxlength="63" pattern={namePattern(server?.engine)} readonly={server?.engine === 'redis'} required bind:value={form.database_name} />{/snippet}</FormField>
	</div>
	<FormField label={t('col.destination')} help={t('help.destinationAgent')}>{#snippet children(a)}<Select {...a} required bind:value={form.destination_id} options={usable.map((d) => ({ value: d.id, label: d.name, brand: kinds[d.kind].brand, description: d.kind === 's3' ? `s3://${d.bucket}/${d.prefix}` : `${d.endpoint}/${d.prefix}` }))} />{/snippet}</FormField>
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
	{/if}
	<div class="flex justify-end gap-2">
		<a class="btn" href={resolve('backups')}>{t('action.cancel')}</a>
		<button type="submit" class="btn btn-primary" disabled={kind === 'incremental' && !base}>{t('action.save')}</button>
	</div>
</form>
