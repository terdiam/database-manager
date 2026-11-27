import { jobs, type Job } from './api/index.ts';

export type Notice = { id: string; job: Job; read: boolean };

const finished = (job: Job) => job.status === 'completed' || job.status === 'failed';
const notice = (job: Job, read: boolean): Notice => ({ id: job.id, job, read });

// Sample data: the newest finished jobs, the last few unread.
export const notices = $state(jobs.filter(finished).slice(0, 6).map((job, i) => notice(job, i > 2)));

/** One job "finishes" while the app is open, to show the live behaviour. */
export function simulateArrival() {
	if (notices.some((n) => n.id === 'j-2041')) return;
	const running = jobs.find((j) => j.id === 'j-2041')!;
	notices.unshift(notice({ ...running, status: 'completed', progress: 100 }, false));
}

export const markAllRead = () => notices.forEach((n) => (n.read = true));
