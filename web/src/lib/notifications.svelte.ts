import { request, type Job, type Page } from './api/http.ts';

const KEY = 'seen-jobs';

function seen(): number {
	try {
		return Number(localStorage.getItem(KEY)) || 0;
	} catch {
		return 0;
	}
}

/** Finished backups and restores, newest first. Read state is per browser. */
export const notices = $state<{ jobs: Job[]; seenAt: number }>({ jobs: [], seenAt: seen() });

export const isUnread = (job: Job) => Date.parse(job.finished_at ?? job.created_at) > notices.seenAt;

export async function refresh() {
	const page = await request<Page<Job>>('GET', '/jobs');
	notices.jobs = page.items
		.filter((j) => (j.kind === 'backup' || j.kind === 'restore') && (j.status === 'completed' || j.status === 'failed'))
		.slice(0, 10);
}

export function markAllRead() {
	notices.seenAt = Date.now();
	try {
		localStorage.setItem(KEY, String(notices.seenAt));
	} catch {
		/* storage blocked: read state lasts for this tab */
	}
}
