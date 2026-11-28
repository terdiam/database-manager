import { request, type Job, type Page } from './api/http.ts';

type Read = { read_at: string };

/** Finished backups and restores, newest first, and up to when this account has read them. */
/** `arrived` goes up each time a result shows up that was not there before. */
export const notices = $state<{ jobs: Job[]; seenAt: number; arrived: number }>({ jobs: [], seenAt: Number.MAX_SAFE_INTEGER, arrived: 0 });
let loaded = false;

const finished = (job: Job) => job.finished_at ?? job.created_at;
export const isUnread = (job: Job) => Date.parse(finished(job)) > notices.seenAt;

export async function refresh() {
	const [page, read] = await Promise.all([request<Page<Job>>('GET', '/jobs'), request<Read>('GET', '/notifications/read')]);
	const jobs = page.items
		.filter((j) => (j.kind === 'backup' || j.kind === 'restore') && (j.status === 'completed' || j.status === 'failed'))
		.slice(0, 10);
	notices.seenAt = Date.parse(read.read_at);
	// The first load is history, not news.
	const known = new Set(notices.jobs.map((j) => j.id));
	if (loaded && jobs.some((j) => !known.has(j.id) && isUnread(j))) notices.arrived++;
	loaded = true;
	notices.jobs = jobs;
}

export async function markAllRead() {
	// Only what is on screen: a job finishing right now stays unread.
	const newest = notices.jobs.map(finished).sort((a, b) => Date.parse(b) - Date.parse(a))[0];
	if (!newest) return;
	const before = notices.seenAt;
	notices.seenAt = Date.parse(newest);
	try {
		notices.seenAt = Date.parse((await request<Read>('POST', '/notifications/read', { read_at: newest })).read_at);
	} catch {
		// Not saved: show it as it is on the server, so it can be tried again.
		notices.seenAt = before;
	}
}
