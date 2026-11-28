<script lang="ts">
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import { ApiError, request, type DbServer, type Job, type Queued, type Snapshot } from '#lib/api/http.ts';
	import ConfirmDialog from '#lib/components/ConfirmDialog.svelte';
	import EmptyState from '#lib/components/EmptyState.svelte';
	import FormField from '#lib/components/FormField.svelte';
	import JobDialog from '#lib/components/JobDialog.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import Select from '#lib/components/Select.svelte';
	import SplitTitle from '#lib/components/SplitTitle.svelte';
	import { namePattern, targetField } from '#lib/api/index.ts';
	import { errorText } from '#lib/errors.ts';
	import { bytes, dateTime } from '#lib/format.ts';
	import { t } from '#lib/i18n/index.svelte.ts';
	import { live } from '#lib/events.ts';
	import { canOperate, isAdmin } from '#lib/session.svelte.ts';

	let snapshot = $state<Snapshot>();
	let servers = $state<DbServer[]>([]);
	let missing = $state(false);
	let error = $state('');
	let target = $state('');
	let database = $state('');
	let restoreConfirm = $state<ConfirmDialog>();
	let deleteConfirm = $state<ConfirmDialog>();
	let job: JobDialog;
	let link = $state('');

	async function download() {
		link = '';
		await queue('POST', `/snapshots/${snapshot!.id}/download`);
	}

	// The link can be fetched once, right after the agent made it.
	async function finished(done: Job) {
		if (done.kind !== 'presign_snapshot' || done.status !== 'completed') return;
		try {
			const { url } = await request<{ url: string }>('GET', `/downloads/${done.id}`);
			if (/^https?:/.test(url)) link = url;
			job.close(); // the button to save the file is behind the dialog
		} catch (e) {
			error = errorText(e);
		}
	}

	let until = $state('');
	/** The value a `datetime-local` field takes, in the browser's own time zone. */
	function localInput(iso: string, addSeconds = 0): string {
		const at = new Date(Date.parse(iso) + addSeconds * 1000);
		return new Date(at.getTime() - at.getTimezoneOffset() * 60_000).toISOString().slice(0, 19);
	}

	// Restores run on the agent that can open the destination's credentials.
	const targets = $derived(servers.filter((s) => s.agent_id === snapshot?.agent_id && s.engine === snapshot?.engine));
	const targetName = $derived(servers.find((s) => s.id === target)?.name ?? '');

	$effect(() => {
		const id = page.params.id;
		Promise.all([request<Snapshot>('GET', `/snapshots/${id}`), request<DbServer[]>('GET', '/database-servers')]).then(
			([s, list]) => {
				missing = false;
				snapshot = s;
				servers = list;
				database =
					// A whole instance or all indices go back under their own names.
					s.engine === 'redis' || s.database_name === '_all' ? s.database_name
					: s.engine === 'sqlite' ? `restore-${s.database_name}`.slice(0, 63)
					: `${s.database_name}_restore`.slice(0, 63);
			},
			(e) => {
				missing = true;
				error = errorText(e);
			}
		);
	});

	// Picks up a deletion or a newly added server without touching what was typed.
	async function reload() {
		const id = page.params.id;
		try {
			const [s, list] = await Promise.all([request<Snapshot>('GET', `/snapshots/${id}`), request<DbServer[]>('GET', '/database-servers')]);
			if (id !== page.params.id) return;
			snapshot = s;
			servers = list;
		} catch (e) {
			if (e instanceof ApiError && e.code === 'not_found') {
				missing = true;
				error = errorText(e);
			}
		}
	}
	onMount(() => live(['snapshots', 'database_servers'], () => missing || reload()));

	async function queue(method: string, path: string, body?: unknown) {
		try {
			job.show((await request<Queued>(method, path, body)).job_id);
			error = '';
		} catch (e) {
			error = errorText(e);
		}
	}
</script>

{#if missing}
	<EmptyState title={t('snapshots.missing')} hint={error} />
{:else if snapshot}
	<PageHeader title="{snapshot.server_name} / {snapshot.database_name}" description={dateTime(snapshot.created_at)}>
		{#snippet actions()}{#if isAdmin()}<button type="button" class="btn btn-danger" onclick={() => deleteConfirm?.open()}>{t('action.delete')}</button>{/if}{/snippet}
	</PageHeader>
	<p class="mb-3 text-sm text-bad empty:hidden" role="alert">{error}</p>

	<section class="card">
		<SplitTitle text={t('snapshots.detail')} class="mb-3 font-semibold" />
		<dl class="grid grid-cols-[9rem_1fr] gap-2 text-sm">
			<dt class="text-dim">{t('col.type')}</dt><dd>{t(snapshot.parent_id ? 'type.incremental' : 'type.full')}</dd>
			<dt class="text-dim">{t('col.size')}</dt><dd>{bytes(snapshot.size)}</dd>
			<dt class="text-dim">{t('col.destination')}</dt><dd>{snapshot.destination}</dd>
			<dt class="text-dim">{t('snapshots.file')}</dt><dd><code class="text-xs break-all">{snapshot.object_key}</code></dd>
			<dt class="text-dim">{t('wizard.compression')}</dt><dd>{snapshot.compression} · age</dd>
			<dt class="text-dim">SHA-256</dt><dd><code class="text-xs break-all">{snapshot.sha256}</code></dd>
			<dt class="text-dim">{t('col.expires')}</dt><dd>{snapshot.expires_at ? dateTime(snapshot.expires_at) : '-'}</dd>
		</dl>
		{#if canOperate() && snapshot.storage === 's3'}
			<div class="mt-4 flex flex-wrap items-center gap-2">
				<button type="button" class="btn" onclick={download}>{t('action.download')}</button>
				<span role="status">{#if link}<a class="btn btn-primary" href={link} target="_blank" rel="noopener noreferrer external">{t('snapshots.saveFile')}</a>{/if}</span>
				<p class="w-full text-xs text-dim">{t('snapshots.downloadHint')}</p>
			</div>
		{/if}
	</section>

	{#if canOperate()}
		<section class="card mt-3">
			<SplitTitle text={t('restore.title')} class="mb-1 font-semibold" />
			<form onsubmit={(e) => { e.preventDefault(); restoreConfirm?.open(); }}>
				<FormField label={t('restore.server')}>{#snippet children(a)}<Select {...a} required bind:value={target} placeholder={t('restore.choose')} options={targets.map((s) => ({ value: s.id, label: s.name, brand: s.engine, description: s.engine === 'sqlite' ? `/${s.host}` : `${s.host}:${s.port}` }))} />{/snippet}</FormField>
				<FormField label={t(snapshot.engine === 'redis' || snapshot.engine === 'elasticsearch' || snapshot.engine === 'opensearch' ? targetField(snapshot.engine).label : 'restore.database')} help={t(snapshot.engine === 'redis' ? 'restore.redisHint' : snapshot.database_name === '_all' ? 'restore.allHint' : snapshot.parent_id ? 'restore.chainHint' : 'restore.createHint')}>{#snippet children(a)}<input {...a} class="input min-h-11" maxlength="63" pattern={namePattern(snapshot?.engine)} readonly={snapshot?.engine === 'redis' || snapshot?.database_name === '_all'} required bind:value={database} />{/snippet}</FormField>
				{#if snapshot.parent_id && snapshot.chain_started_at}
					<FormField label={t('restore.until')} help={t('restore.untilHint')}>{#snippet children(a)}<input {...a} class="input min-h-11" type="datetime-local" step="1" min={localInput(snapshot!.chain_started_at!, 1)} max={localInput(snapshot!.created_at)} bind:value={until} />{/snippet}</FormField>
				{/if}
				<button type="submit" class="btn btn-primary">{t('restore.start')}</button>
			</form>
		</section>
	{/if}

	<ConfirmDialog bind:this={restoreConfirm} title={t('restore.title')} message={t(snapshot?.engine === 'redis' ? 'restore.redisWarning' : database === '_all' ? 'restore.allWarning' : snapshot?.parent_id ? 'restore.chainWarning' : 'restore.warning', { database, server: targetName })} confirmLabel={t('restore.start')} expect={database}
		onconfirm={() => queue('POST', `/snapshots/${snapshot!.id}/restore`, { server_id: target, database_name: database, until: until ? new Date(until).toISOString() : undefined })} />
	<ConfirmDialog bind:this={deleteConfirm} title={t('action.delete')} message={t('snapshots.confirmDelete')} confirmLabel={t('action.delete')} onconfirm={() => queue('DELETE', `/snapshots/${snapshot!.id}`)} />
{/if}

<JobDialog bind:this={job} title={t('action.log')} onfinish={finished} />
