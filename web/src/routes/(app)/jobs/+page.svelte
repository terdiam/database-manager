<script lang="ts">
	import IconButton from '#lib/components/IconButton.svelte';
	import { onMount } from 'svelte';
	import { request, type Job, type Page } from '#lib/api/http.ts';
	import DataTable from '#lib/components/DataTable.svelte';
	import DateFilter from '#lib/components/DateFilter.svelte';
	import JobMonitor from '#lib/components/JobMonitor.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import Select from '#lib/components/Select.svelte';
	import SplitTitle from '#lib/components/SplitTitle.svelte';
	import StatusBadge from '#lib/components/StatusBadge.svelte';
	import { dateParams, emptyDate } from '#lib/daterange.ts';
	import { errorText } from '#lib/errors.ts';
	import { bytes, dateTime } from '#lib/format.ts';
	import { t } from '#lib/i18n/index.svelte.ts';
	import { live } from '#lib/events.ts';

	let period = $state(emptyDate());
	let status = $state('');
	let rows = $state<Job[]>([]);
	let next = $state<string | null>(null);
	let selected = $state('');
	let error = $state('');
	let paged = false; // older pages are on screen; a refresh would drop them

	const filter = $derived.by(() => {
		const params = dateParams(period);
		if (status) params.set('status', status);
		return params.toString();
	});

	async function load(query: string, before: string | null) {
		paged = before !== null;
		try {
			const page = await request<Page<Job>>('GET', `/jobs?${query}${before ? `&before=${before}` : ''}`);
			if (query !== filter) return;
			rows = before ? [...rows, ...page.items] : page.items;
			next = page.next;
			selected ||= page.items[0]?.id ?? '';
			error = '';
		} catch (e) {
			error = errorText(e);
		}
	}
	$effect(() => void load(filter, null));

	onMount(() => live(['jobs'], () => paged || load(filter, null)));
</script>

<PageHeader title={t('nav.jobs')} description={t('jobs.desc')} />
<p class="mb-3 text-sm text-bad empty:hidden" role="alert">{error}</p>

<search class="mb-3 flex flex-wrap items-center gap-2">
	<DateFilter bind:value={period} />
	<Select class="w-48" aria-label={t('col.status')} bind:value={status} options={[{ value: '', label: t('filter.allStatus') }, ...(['completed', 'failed', 'running', 'pending', 'cancelled'] as const).map((s) => ({ value: s, label: t(`status.${s}`) }))]} />
</search>

<DataTable label={t('nav.jobs')} columns={[t('col.started'), t('col.type'), t('col.target'), t('col.status'), t('col.size'), t('audit.user'), t('col.actions')]} {rows}>
	{#snippet cell(job, i)}
		{#if i === 0}<span class="whitespace-nowrap">{dateTime(job.created_at)}</span>
		{:else if i === 1}{t(`job.${job.kind}`)}
		{:else if i === 2}{job.target}
		{:else if i === 3}<StatusBadge status={job.status} />
		{:else if i === 4}{job.bytes ? bytes(job.bytes) : '-'}
		{:else if i === 5}{job.created_by}
		{:else}<IconButton icon="list" label={t('action.log')} onclick={() => (selected = job.id)} pressed={selected === job.id} />{/if}
	{/snippet}
</DataTable>

{#if next}<button type="button" class="btn mt-3" onclick={() => load(filter, next)}>{t('action.more')}</button>{/if}

{#if selected}
	<section class="card mt-3">
		<SplitTitle text={t('action.log')} class="mb-2 font-semibold" />
		<JobMonitor jobId={selected} onfinish={() => load(filter, null)} />
	</section>
{/if}
