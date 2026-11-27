<script lang="ts">
	import SplitTitle from '#lib/components/SplitTitle.svelte';
	import { resolve } from '$app/paths';
	import { activity, agents, destinations, jobs } from '#lib/api/index.ts';
	import ActivityChart, { type Series } from '#lib/components/ActivityChart.svelte';
	import DataTable from '#lib/components/DataTable.svelte';
	import DonutChart from '#lib/components/DonutChart.svelte';
	import Icon from '#lib/components/Icon.svelte';
	import JobStatusGrid from '#lib/components/JobStatusGrid.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import ProgressBar from '#lib/components/ProgressBar.svelte';
	import StatCard from '#lib/components/StatCard.svelte';
	import StatusBadge from '#lib/components/StatusBadge.svelte';
	import { bytes, dateTime } from '#lib/format.ts';
	import { t } from '#lib/i18n/index.svelte.ts';

	const [ok, bad] = activity[activity.length - 1];
	const storage = destinations.reduce((sum, d) => sum + d.used, 0);
	const online = agents.filter((a) => a.status === 'online').length;
	const running = jobs.filter((j) => j.status === 'running');
	// The colour picked in the chart also filters the recent jobs below.
	let filter = $state<Series | ''>('');
	const recent = $derived(jobs.filter((j) => !filter || j.status === filter).slice(0, 5));
	const attention = [
		{ text: 'attention.failed', name: 'prod-mysql', href: 'jobs' },
		{ text: 'attention.agent', name: 'branch-surabaya', href: 'agents' },
		{ text: 'attention.destination', name: 'arsip-drive', href: 'destinations' }
	] as const;
</script>

<PageHeader title={t('nav.dashboard')} description={t('dashboard.desc')}>
	{#snippet actions()}<a class="btn btn-primary" href={resolve('backups/new')}>{t('backups.new')}</a>{/snippet}
</PageHeader>

<div class="grid grid-cols-2 gap-3 xl:grid-cols-4">
	<StatCard icon="check" tone="text-ok" label={t('dashboard.success')} value={ok} hint={t('dashboard.last24')} />
	<StatCard icon="alert" tone="text-bad" label={t('dashboard.failed')} value={bad} hint={t('dashboard.last24')} />
	<StatCard icon="cloud" label={t('dashboard.storage')} value={bytes(storage)} hint={t('dashboard.destinations', { n: destinations.length })} />
	<StatCard icon="server" label={t('dashboard.agents')} value="{online}/{agents.length}" hint={t('status.online')} />
</div>

<div class="mt-3 grid gap-3">
	<ActivityChart data={activity} bind:filter />
	<section class="card">
		<SplitTitle text={t('dashboard.attention')} class="mb-3 font-semibold" />
		<ul class="grid gap-2 text-sm md:grid-cols-3">
			{#each attention as item}
				<li><a class="block h-full rounded-lg border border-line p-2 hover:bg-muted" href={resolve(item.href)}><strong class="text-bad">{item.name}</strong><br />{t(item.text)}</a></li>
			{/each}
		</ul>
	</section>
</div>

<div class="mt-3 grid gap-3">
	<JobStatusGrid />
	<DonutChart title={t('dashboard.storageByVolume')} items={destinations.map((d) => ({ name: d.name, value: d.used }))} />
</div>

<section class="card mt-3">
	<SplitTitle text={t('dashboard.running')} class="mb-3 font-semibold" />
	{#each running as job}
		<div class="grid items-center gap-2 text-sm sm:grid-cols-[10rem_1fr_auto]">
			<span class="font-medium">{job.target} <span class="text-dim">{job.id}</span></span>
			<ProgressBar value={job.progress} label="{job.target} {job.id}" />
			<a class="btn" href={resolve('jobs')}>{t('action.log')}</a>
		</div>
	{:else}<p class="text-sm text-dim">{t('dashboard.idle')}</p>{/each}
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
<DataTable label={t('dashboard.recent')} columns={[t('col.job'), t('col.type'), t('col.target'), t('col.status'), t('col.started')]} rows={recent}>
	{#snippet cell(job, i)}
		{#if i === 0}{job.id}{:else if i === 1}{t(`job.${job.type}`)}{:else if i === 2}{job.target}{:else if i === 3}<StatusBadge status={job.status} />{:else}{dateTime(job.started)}{/if}
	{/snippet}
</DataTable>
