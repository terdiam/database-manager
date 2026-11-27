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
/** `arrived` goes up each time a result shows up that was not there before. */
export const notices = $state<{ jobs: Job[]; seenAt: number; arrived: number }>({ jobs: [], seenAt: seen(), arrived: 0 });
let loaded = false;

export const isUnread = (job: Job) => Date.parse(job.finished_at ?? job.created_at) > notices.seenAt;

export async function refresh() {
	const page = await request<Page<Job>>('GET', '/jobs');
	const jobs = page.items
		.filter((j) => (j.kind === 'backup' || j.kind === 'restore') && (j.status === 'completed' || j.status === 'failed'))
		.slice(0, 10);
	// The first load is history, not news.
	const known = new Set(notices.jobs.map((j) => j.id));
	if (loaded && jobs.some((j) => !known.has(j.id) && isUnread(j))) notices.arrived++;
	loaded = true;
	notices.jobs = jobs;
}

export function markAllRead() {
	notices.seenAt = Date.now();
	try {
		localStorage.setItem(KEY, String(notices.seenAt));
	} catch {
		/* storage blocked: read state lasts for this tab */
	}
}
