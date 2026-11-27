<script lang="ts">
	import { onMount } from 'svelte';
	import { resolve } from '$app/paths';
	import { request, type Backup, type Queued } from '#lib/api/http.ts';
	import ConfirmDialog from '#lib/components/ConfirmDialog.svelte';
	import DataTable from '#lib/components/DataTable.svelte';
	import JobDialog from '#lib/components/JobDialog.svelte';
	import Modal from '#lib/components/Modal.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import RetentionPicker from '#lib/components/RetentionPicker.svelte';
	import SchedulePicker from '#lib/components/SchedulePicker.svelte';
	import { toast } from '#lib/components/toast.svelte.ts';
	import { errorText } from '#lib/errors.ts';
	import { dateTime } from '#lib/format.ts';
	import { locale, t } from '#lib/i18n/index.svelte.ts';
	import { fromCron, localZone, newRetention, newSchedule, retentionOf, toPlan } from '#lib/schedule.ts';
	import { live } from '#lib/events.ts';
	import { canOperate } from '#lib/session.svelte.ts';

	let backups = $state<Backup[]>([]);
	let error = $state('');
	let loadError = $state('');
	let removing = $state<Backup>();
	let confirm: ConfirmDialog;
	let job: JobDialog;
	let editor: Modal;
	let editing = $state<Backup>();
	let schedule = $state(newSchedule('manual'));
	let timezone = $state(localZone());
	let retention = $state(newRetention());
	let editError = $state('');

	function edit(backup: Backup) {
		editing = backup;
		schedule = fromCron(backup.schedule);
		timezone = backup.schedule ? backup.timezone : localZone();
		retention = retentionOf(backup);
		editError = '';
		editor.open();
	}

	async function save(close: () => void) {
		try {
			await request('PUT', `/backups/${editing!.id}`, toPlan(schedule, timezone, retention));
			close();
			toast(t('toast.updated', { name: editing!.name }));
			await load();
		} catch (e) {
			editError = errorText(e);
		}
	}

	function scheduleText(backup: Backup): string {
		const s = fromCron(backup.schedule);
		if (s.kind === 'manual') return t('schedule.manual');
		if (s.kind === 'cron') return s.cron;
		if (s.kind === 'hours') return t('schedule.everyHours', { n: s.every });
		const day =
			s.kind === 'weekly'
				? new Intl.DateTimeFormat(locale.current, { weekday: 'long' }).format(new Date(2026, 1, 1 + s.weekday))
				: s.kind === 'monthly'
					? String(s.day)
					: s.kind === 'yearly'
						? new Intl.DateTimeFormat(locale.current, { day: 'numeric', month: 'long' }).format(new Date(2026, s.month - 1, s.day))
						: '';
		return `${t(`schedule.${s.kind}`)} ${day} ${s.time}`.replace('  ', ' ');
	}

	function retentionText(b: Backup): string {
		if (b.retention_unit === 'gfs') return `GFS ${b.keep_daily}/${b.keep_weekly}/${b.keep_monthly}`;
		if (!b.retention_unit) return t('retention.all');
		return `${b.retention_value} ${t(`retention.${b.retention_unit}`).toLowerCase()}`;
	}

	const load = () =>
		request<Backup[]>('GET', '/backups').then(
			(list) => {
				backups = list;
				loadError = '';
			},
			(e) => void (loadError = errorText(e))
		);
	onMount(() => {
		load();
		return live(['backup_configs', 'database_servers', 'destinations'], load);
	});

	async function run(backup: Backup) {
		try {
			job.show((await request<Queued>('POST', `/backups/${backup.id}/run`)).job_id);
			error = '';
		} catch (e) {
			error = errorText(e);
		}
	}

	async function remove() {
		try {
			await request('DELETE', `/backups/${removing!.id}`);
			toast(t('toast.deleted', { name: removing!.name }));
			await load();
		} catch (e) {
			error = errorText(e);
		}
	}
</script>

<PageHeader title={t('nav.backups')} description={t('backups.desc')}>
	{#snippet actions()}{#if canOperate()}<a class="btn btn-primary" href={resolve('backups/new')}>{t('backups.new')}</a>{/if}{/snippet}
</PageHeader>

<p class="mb-3 text-sm text-bad empty:hidden" role="alert">{error || loadError}</p>

<DataTable label={t('nav.backups')} columns={[t('col.name'), t('col.source'), t('col.destination'), t('col.schedule'), t('retention.title'), ...(canOperate() ? [t('col.actions')] : [])]} rows={backups}>
	{#snippet cell(b, i)}
		{#if i === 0}<strong>{b.name}</strong>
		{:else if i === 1}{b.server} / {b.database_name}
		{:else if i === 2}{b.destination}
		{:else if i === 3}
			{scheduleText(b)}
			{#if b.next_run_at}<span class="block text-xs text-dim">{t('schedule.next')}: {dateTime(b.next_run_at)}</span>{/if}
		{:else if i === 4}{retentionText(b)}
		{:else}<span class="flex flex-wrap justify-end gap-1">
			<button type="button" class="btn" onclick={() => run(b)}>{t('action.runNow')}</button>
			<button type="button" class="btn" onclick={() => edit(b)}>{t('action.schedule')}</button>
			<button type="button" class="btn btn-danger" onclick={() => { removing = b; confirm.open(); }}>{t('action.delete')}</button>
		</span>{/if}
	{/snippet}
</DataTable>

<Modal bind:this={editor} title="{t('action.schedule')}: {editing?.name ?? ''}">
	{#snippet children(close)}
		<form onsubmit={(e) => { e.preventDefault(); save(close); }}>
			<SchedulePicker legend={t('col.schedule')} bind:value={schedule} bind:timezone />
			<RetentionPicker bind:value={retention} />
			<p class="mb-3 text-xs text-dim">{t('retention.fromNow')}</p>
			<p class="text-sm text-bad empty:hidden" role="alert">{editError}</p>
			<div class="mt-4 flex justify-end gap-2">
				<button type="button" class="btn" onclick={close}>{t('action.cancel')}</button>
				<button type="submit" class="btn btn-primary">{t('action.save')}</button>
			</div>
		</form>
	{/snippet}
</Modal>

<JobDialog bind:this={job} title={t('job.backup')} />
<ConfirmDialog bind:this={confirm} title={t('action.delete')} message={t('confirm.delete', { name: removing?.name ?? '' })} confirmLabel={t('action.delete')} onconfirm={remove} />
