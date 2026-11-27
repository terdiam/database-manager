<script lang="ts">
	import { onMount } from 'svelte';
	import { resolve } from '$app/paths';
	import { request, type Dashboard } from '#lib/api/http.ts';
	import ActivityChart, { type Series } from '#lib/components/ActivityChart.svelte';
	import DataTable from '#lib/components/DataTable.svelte';
	import DonutChart from '#lib/components/DonutChart.svelte';
	import Icon from '#lib/components/Icon.svelte';
	import JobStatusGrid from '#lib/components/JobStatusGrid.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import SplitTitle from '#lib/components/SplitTitle.svelte';
	import StatCard from '#lib/components/StatCard.svelte';
	import StatusBadge from '#lib/components/StatusBadge.svelte';
	import { errorText } from '#lib/errors.ts';
	import { live } from '#lib/events.ts';
	import { bytes, dateTime } from '#lib/format.ts';
	import { t } from '#lib/i18n/index.svelte.ts';
	import { localZone } from '#lib/schedule.ts';
	import { canOperate } from '#lib/session.svelte.ts';

	let data = $state<Dashboard>();
	let error = $state('');
	// The colour picked in the chart also filters the recent jobs below.
	let filter = $state<Series | ''>('');

	async function load() {
		try {
			data = await request<Dashboard>('GET', `/dashboard?${new URLSearchParams({ timezone: localZone() })}`);
			error = '';
		} catch (e) {
			error = errorText(e);
		}
	}
	onMount(() => {
		load();
		return live(['jobs', 'agents', 'snapshots', 'destinations'], load, { every: 30_000 });
	});

	const online = $derived(data?.agents.filter((a) => a.online).length ?? 0);
	const storage = $derived(data?.volumes.reduce((sum, v) => sum + v.bytes, 0) ?? 0);
	const running = $derived(data?.jobs.filter((j) => j.status === 'running' || j.status === 'pending') ?? []);
	const recent = $derived(data?.jobs.filter((j) => !filter || j.status === filter).slice(0, 5) ?? []);
	const attention = $derived([
		...(data?.failing.map((f) => ({ key: `b-${f.name}`, name: f.name, text: t('attention.failed'), href: 'jobs' as const })) ?? []),
		...(data?.agents.filter((a) => !a.online).map((a) => ({ key: `a-${a.name}`, name: a.name, text: t('attention.agent'), href: 'agents' as const })) ?? [])
	]);
</script>

<PageHeader title={t('nav.dashboard')} description={t('dashboard.desc')}>
	{#snippet actions()}{#if canOperate()}<a class="btn btn-primary" href={resolve('backups/new')}>{t('backups.new')}</a>{/if}{/snippet}
</PageHeader>

<p class="mb-3 text-sm text-bad empty:hidden" role="alert">{error}</p>

{#if data}
	<div class="grid grid-cols-2 gap-3 xl:grid-cols-4">
		<StatCard icon="check" tone="text-ok" label={t('dashboard.success')} value={data.completed_24h} hint={t('dashboard.last24')} />
		<StatCard icon="alert" tone="text-bad" label={t('dashboard.failed')} value={data.failed_24h} hint={t('dashboard.last24')} />
		<StatCard icon="cloud" label={t('dashboard.storage')} value={bytes(storage)} hint={t('dashboard.destinations', { n: data.volumes.length })} />
		<StatCard icon="server" label={t('dashboard.agents')} value="{online}/{data.agents.length}" hint={t('status.online')} />
	</div>

	<div class="mt-3 grid gap-3">
		<ActivityChart data={data.activity.map((d) => [d.completed, d.failed])} bind:filter />
		<section class="card">
			<SplitTitle text={t('dashboard.attention')} class="mb-3 font-semibold" />
			<ul class="grid gap-2 text-sm md:grid-cols-3">
				{#each attention as item (item.key)}
					<li><a class="block h-full rounded-lg border border-line p-2 hover:bg-muted" href={resolve(item.href)}><strong class="text-bad">{item.name}</strong><br />{item.text}</a></li>
				{:else}
					<li class="flex items-center gap-2 text-dim"><Icon name="check" class="text-ok" />{t('attention.none')}</li>
				{/each}
			</ul>
		</section>
	</div>

	<div class="mt-3 grid gap-3">
		<JobStatusGrid jobs={[...data.jobs].reverse()} />
		<DonutChart title={t('dashboard.storageByVolume')} items={data.volumes.map((v) => ({ name: v.name, value: v.bytes }))} />
	</div>

	<section class="card mt-3">
		<SplitTitle text={t('dashboard.running')} class="mb-3 font-semibold" />
		<ul class="grid gap-2 text-sm">
			{#each running as job (job.id)}
				<li class="flex flex-wrap items-center gap-2">
					<StatusBadge status={job.status} />
					<span class="font-medium">{t(`job.${job.kind}`)} · {job.target}</span>
					{#if job.bytes}<span class="tabular-nums">{bytes(job.bytes)}</span>{/if}
					<a class="btn ml-auto" href={resolve('jobs')}>{t('action.log')}</a>
				</li>
			{:else}
				<li class="text-dim">{t('dashboard.idle')}</li>
			{/each}
		</ul>
	</section>

	<div class="mt-5 mb-2 flex flex-wrap items-center gap-2">
		<SplitTitle text={t('dashboard.recent')} class="font-semibold" />
		<span class="contents" role="status">
			{#if filter}
				<button type="button" class="btn min-h-8 text-xs" onclick={() => (filter = '')}>
					{t('chart.filtered', { status: t(`status.${filter}`) })}<Icon name="close" size={14} />
				</button>
			{/if}
		</span>
	</div>
	<DataTable label={t('dashboard.recent')} columns={[t('col.started'), t('col.type'), t('col.target'), t('col.status'), t('col.size')]} rows={recent}>
		{#snippet cell(job, i)}
			{#if i === 0}{dateTime(job.created_at)}{:else if i === 1}{t(`job.${job.kind}`)}{:else if i === 2}{job.target}{:else if i === 3}<StatusBadge status={job.status} />{:else}{job.bytes ? bytes(job.bytes) : '-'}{/if}
		{/snippet}
	</DataTable>
{:else if !error}
	<p class="text-sm text-dim">{t('login.checking')}</p>
{/if}
