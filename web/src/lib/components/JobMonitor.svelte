<script lang="ts">
	import { request, type Job, type JobLine } from '#lib/api/http.ts';
	import { errorText } from '#lib/errors.ts';
	import { live } from '#lib/events.ts';
	import { bytes } from '#lib/format.ts';
	import { t } from '#lib/i18n/index.svelte.ts';
	import { canOperate } from '#lib/session.svelte.ts';
	import LogViewer from './LogViewer.svelte';
	import StatusBadge from './StatusBadge.svelte';

	// Follows one job until it finishes: each change event fetches only the new log lines,
	// through the same (optionally encrypted) API as everything else.
	let { jobId, onfinish }: { jobId: string; onfinish?: (job: Job) => void } = $props();
	let job = $state<Job>();
	let lines = $state<string[]>([]);
	let error = $state('');

	$effect(() => {
		const id = jobId;
		let after = 0;
		let stopped = false;
		let finished = false;
		job = undefined;
		lines = [];

		async function load() {
			if (finished) return;
			try {
				let reply: Job & { logs: JobLine[] };
				// A full page means more lines are waiting.
				do {
					reply = await request<Job & { logs: JobLine[] }>('GET', `/jobs/${id}?after=${after}`);
					if (stopped) return;
					error = '';
					job = reply;
					if (reply.logs.length) {
						after = reply.logs[reply.logs.length - 1].id;
						lines = [...lines, ...reply.logs.map((l) => `[${l.level}] ${l.message}`)];
					}
				} while (reply.logs.length === 500);
				if (reply.status !== 'pending' && reply.status !== 'running') {
					finished = true;
					stop();
					onfinish?.(reply);
				}
			} catch (e) {
				if (!stopped) error = errorText(e);
			}
		}
		const stop = live(['jobs'], load, { every: 15_000, now: true });
		return () => {
			stopped = true;
			stop();
		};
	});

	async function cancel() {
		try {
			await request('POST', `/jobs/${jobId}/cancel`);
		} catch (e) {
			error = errorText(e);
		}
	}
</script>

<p class="mb-2 text-sm text-bad empty:hidden" role="alert">{error}</p>
{#if job}
	<div class="mb-2 flex flex-wrap items-center gap-2 text-sm" role="status">
		<StatusBadge status={job.status} />
		<strong>{t(`job.${job.kind}`)}</strong><span class="text-dim">{job.target} · {job.agent}</span>
		{#if job.bytes}<span class="tabular-nums">{bytes(job.bytes)}</span>{/if}
		{#if canOperate() && (job.status === 'pending' || job.status === 'running')}
			<button type="button" class="btn btn-danger ml-auto min-h-8 text-xs" disabled={job.cancel_requested} onclick={cancel}>{t('jobs.cancel')}</button>
		{/if}
	</div>
	{#if job.error}<p class="mb-2 rounded-lg border border-bad p-3 text-sm">{job.error}</p>{/if}
	<LogViewer lines={lines.length ? lines : [t(job.status === 'pending' ? 'jobs.waiting' : 'jobs.noLog')]} label={t('action.log')} />
{:else if !error}
	<p class="text-sm text-dim">{t('login.checking')}</p>
{/if}
