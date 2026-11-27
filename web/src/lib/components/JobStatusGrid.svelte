<script lang="ts">
	import SplitTitle from '#lib/components/SplitTitle.svelte';
	import { resolve } from '$app/paths';
	import type { Status } from '#lib/api/index.ts';
	import type { Dashboard } from '#lib/api/http.ts';
	import { dateTime } from '#lib/format.ts';
	import { t } from '#lib/i18n/index.svelte.ts';

	/** Oldest first. */
	let { jobs }: { jobs: Dashboard['jobs'] } = $props();

	// Symbol + color: status never depends on color alone.
	const look: Partial<Record<Status, [string, string]>> = {
		completed: ['bg-ok', ''],
		failed: ['bg-bad', '✕'],
		running: ['bg-brand', '▶'],
		pending: ['bg-warn', '…'],
		cancelled: ['bg-dim', '–']
	};
	const statuses = Object.keys(look) as Status[];
	const count = (status: Status) => jobs.filter((j) => j.status === status).length;
</script>

<section class="card">
	<SplitTitle text={t('dashboard.jobStatus')} class="font-semibold" />
	<p class="mb-3 text-xs text-dim">{t('dashboard.jobStatusHint', { n: jobs.length })}</p>
	<ul class="grid grid-cols-[repeat(auto-fill,minmax(1.5rem,1fr))] gap-0.5">
		{#each jobs as job (job.id)}
			{@const label = `${t(`job.${job.kind}`)} · ${job.target} · ${t(`status.${job.status}`)} · ${dateTime(job.created_at)}`}
			<li>
				<a href={resolve('jobs')} title={label} aria-label={label} class="grid aspect-square min-h-6 place-items-center rounded text-[0.65rem] font-bold text-surface hover:ring-2 hover:ring-ink {look[job.status]![0]}">
					<span aria-hidden="true">{look[job.status]![1]}</span>
				</a>
			</li>
		{/each}
	</ul>
	<ul class="mt-3 flex flex-wrap gap-x-4 gap-y-1 text-xs">
		{#each statuses as status}
			<li class="flex items-center gap-1.5">
				<span class="grid size-4 place-items-center rounded text-[0.6rem] font-bold text-surface {look[status]![0]}" aria-hidden="true">{look[status]![1]}</span>
				{t(`status.${status}`)} <strong class="tabular-nums">{count(status)}</strong>
			</li>
		{/each}
	</ul>
</section>
