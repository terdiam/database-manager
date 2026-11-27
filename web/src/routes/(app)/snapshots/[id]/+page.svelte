<script lang="ts">
	import SplitTitle from '#lib/components/SplitTitle.svelte';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { agents, engines, servers, snapshots } from '#lib/api/index.ts';
	import ChainTimeline from '#lib/components/ChainTimeline.svelte';
	import ConfirmDialog from '#lib/components/ConfirmDialog.svelte';
	import EmptyState from '#lib/components/EmptyState.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import Select from '#lib/components/Select.svelte';
	import StatusBadge from '#lib/components/StatusBadge.svelte';
	import { toast } from '#lib/components/toast.svelte.ts';
	import { bytes, date, dateTime } from '#lib/format.ts';
	import { t } from '#lib/i18n/index.svelte.ts';

	const snapshot = $derived(snapshots.find((s) => s.id === page.params.id));
	const chain = $derived(snapshots.filter((s) => s.chain === snapshot?.chain));
	const points = $derived(chain.filter((s) => s.status === 'completed'));
	// PostgreSQL incremental chains are physical: restore writes a data directory, not a database.
	const physical = $derived(snapshot?.engine === 'postgresql' && chain.some((s) => s.type === 'incremental'));
	const targets = $derived(physical ? agents : servers.filter((s) => s.engine === snapshot?.engine));

	// Writable derived: follows the URL, user can pick another point.
	let point = $derived<string | undefined>(points.some((p) => p.id === page.params.id) ? page.params.id : points[0]?.id);
	let target = $state('');
	let database = $state('');
	let confirm = $state<ConfirmDialog>();

	function submit(event: SubmitEvent) {
		event.preventDefault();
		confirm?.open();
	}
</script>

{#if !snapshot}
	<EmptyState title={t('snapshots.missing')} />
{:else}
	<PageHeader title="Snapshot {snapshot.id}" description="{snapshot.server} · {engines[snapshot.engine].label} · {snapshot.database}">
		{#snippet actions()}<button type="button" class="btn" disabled={snapshot.status !== 'completed'} onclick={() => toast(t('toast.download'))}>{t('action.download')}</button>{/snippet}
	</PageHeader>

	<div class="grid gap-3">
		<section class="card">
			<SplitTitle text={t('snapshots.detail')} class="mb-3 font-semibold" />
			<dl class="grid grid-cols-[9rem_1fr] gap-2 text-sm">
				<dt class="text-dim">{t('col.status')}</dt><dd><StatusBadge status={snapshot.status} /></dd>
				<dt class="text-dim">{t('col.type')}</dt><dd>{t(`type.${snapshot.type}`)}</dd>
				<dt class="text-dim">{t('col.size')}</dt><dd>{bytes(snapshot.size)}</dd>
				<dt class="text-dim">{t('col.created')}</dt><dd>{dateTime(snapshot.created)}</dd>
				<dt class="text-dim">{t('col.expires')}</dt><dd>{date(snapshot.expires)}</dd>
				<dt class="text-dim">{t('col.destination')}</dt><dd>{snapshot.destinations.join(', ')}</dd>
				<dt class="text-dim">Checksum</dt><dd><code class="text-xs break-all">{snapshot.checksum}</code></dd>
			</dl>
		</section>
		<section class="card">
			<SplitTitle text={t('snapshots.chain')} class="mb-3 font-semibold" />
			<ChainTimeline items={chain} current={snapshot.id} />
		</section>
	</div>

	<section class="card mt-3">
		<SplitTitle text={t('restore.title')} class="mb-1 font-semibold" />
		{#if points.length === 0}
			<p class="text-sm text-bad">{t('restore.unavailable')}</p>
		{:else}
			<form onsubmit={submit}>
				<label class="mt-3 block text-sm">{t('restore.point')}
					<Select class="mt-1" bind:value={point} options={points.map((p) => ({ value: p.id, label: dateTime(p.created), icon: 'clock' as const, description: `${t(`type.${p.type}`)} · ${bytes(p.size)}` }))} />
				</label>
				{#if physical}
					<p class="mt-3 rounded-lg bg-muted p-3 text-sm">{t('restore.pgPhysical')}</p>
				{/if}
				<label class="mt-3 block text-sm">{t(physical ? 'restore.agent' : 'restore.server')}
					<Select class="mt-1" required bind:value={target} placeholder={t('restore.choose')} options={targets.map((s) => ({ value: s.name, label: s.name, brand: 'engine' in s ? s.engine : undefined, icon: 'engine' in s ? undefined : ('server' as const), description: t(`status.${s.status}`) }))} />
				</label>
				<label class="mt-3 block text-sm">{t(physical ? 'restore.folder' : 'restore.database')}
					<input class="input mt-1" required bind:value={database} placeholder={physical ? '/var/lib/postgresql/restore' : `${snapshot.database}_restore`} />
				</label>
				<button type="submit" class="btn btn-primary mt-4">{t('restore.start')}</button>
			</form>
		{/if}
	</section>

	<ConfirmDialog
		bind:this={confirm}
		title={t('restore.title')}
		message={t(physical ? 'restore.folderWarning' : 'restore.warning', { database, server: target })}
		confirmLabel={t('restore.start')}
		expect={database}
		onconfirm={() => { toast(t('toast.restore', { name: database })); goto(resolve('jobs')); }}
	/>
{/if}
