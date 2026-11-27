<script lang="ts">
	import { jobLog, jobs } from '#lib/api/index.ts';
	import DateFilter from '#lib/components/DateFilter.svelte';
	import Select from '#lib/components/Select.svelte';
	import { emptyDate, inDate } from '#lib/daterange.ts';
	import DataTable from '#lib/components/DataTable.svelte';
	import LogViewer from '#lib/components/LogViewer.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import ProgressBar from '#lib/components/ProgressBar.svelte';
	import StatusBadge from '#lib/components/StatusBadge.svelte';
	import { dateTime } from '#lib/format.ts';
	import { t } from '#lib/i18n/index.svelte.ts';

	let date = $state(emptyDate());
	let status = $state('');
	const rows = $derived(jobs.filter((j) => inDate(j.started, date) && (!status || j.status === status)));
	let selected = $state(jobs[0]);
	let live = $state(0);

	// Mock of the SSE stream: one extra line every 2s while the selected job runs.
	$effect(() => {
		live = 0;
		if (selected.status !== 'running') return;
		const timer = setInterval(() => (live = Math.min(live + 1, 20)), 2000);
		return () => clearInterval(timer);
	});
	const lines = $derived([...jobLog(selected), ...Array.from({ length: live }, (_, i) => `[info] streamed part ${i + 1}`)]);
</script>

<PageHeader title={t('nav.jobs')} description={t('jobs.desc')} />

<search class="mb-3 flex flex-wrap items-center gap-2">
	<DateFilter bind:value={date} />
	<Select class="w-48" aria-label={t('col.status')} bind:value={status} options={[{ value: '', label: t('filter.allStatus') }, ...(['completed', 'failed', 'running', 'pending'] as const).map((s) => ({ value: s, label: t(`status.${s}`) }))]} />
</search>

<p class="mb-2 text-xs text-dim" role="status">{t('filter.count', { n: rows.length, total: jobs.length })}</p>

<DataTable label={t('nav.jobs')} columns={[t('col.job'), t('col.type'), t('col.target'), t('col.status'), t('col.progress'), t('col.started'), t('col.duration')]} {rows}>
	{#snippet cell(job, i)}
		{#if i === 0}<button type="button" class="font-medium text-brand underline" aria-pressed={selected.id === job.id} onclick={() => (selected = job)}>{job.id}</button>
		{:else if i === 1}{t(`job.${job.type}`)}
		{:else if i === 2}{job.target}
		{:else if i === 3}<StatusBadge status={job.status} />
		{:else if i === 4}<div class="w-32"><ProgressBar value={job.progress} label={job.id} /></div>
		{:else if i === 5}{dateTime(job.started)}
		{:else}{job.duration}{/if}
	{/snippet}
</DataTable>

<section class="card mt-3">
	<h2 class="mb-2 font-semibold">{t('action.log')} {selected.id} · {selected.target}</h2>
	{#if selected.status === 'failed'}<p class="mb-2 rounded-lg border border-bad p-3 text-sm">{t('jobs.hintAccess')}</p>{/if}
	<LogViewer {lines} label="{t('action.log')} {selected.id}" />
</section>
