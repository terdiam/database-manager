export type Schedule = {
	kind: 'hours' | 'daily' | 'weekly' | 'monthly' | 'yearly' | 'cron';
	every: number;
	time: string;
	weekday: number;
	day: number;
	month: number;
	cron: string;
};

export const newSchedule = (kind: Schedule['kind'], patch: Partial<Schedule> = {}): Schedule => ({
	kind,
	every: 6,
	time: '01:00',
	weekday: 0,
	day: 1,
	month: 1,
	cron: '0 1 * * *',
	...patch
});

/** Preview only (browser timezone); the server owns the real next_run_at. Cron returns []. */
export function nextRuns(s: Schedule, count = 3, from = new Date()): Date[] {
	if (s.kind === 'cron') return [];
	const [h, m] = s.time.split(':').map(Number);
	const out: Date[] = [];
	if (s.kind === 'hours') {
		const every = Math.max(1, Math.min(23, s.every));
		const d = new Date(from);
		d.setMinutes(0, 0, 0);
		while (out.length < count) {
			d.setHours(d.getHours() + 1);
			if (d.getHours() % every === 0) out.push(new Date(d));
		}
		return out;
	}
	const d = new Date(from);
	d.setHours(h || 0, m || 0, 0, 0);
	// Walk day by day: simple, and months without the chosen date are skipped like cron does.
	for (let i = 0; out.length < count && i < 366 * (count + 4); i++, d.setDate(d.getDate() + 1)) {
		const match =
			s.kind === 'daily' ||
			(s.kind === 'weekly' && d.getDay() === s.weekday) ||
			(s.kind === 'monthly' && d.getDate() === s.day) ||
			(s.kind === 'yearly' && d.getDate() === s.day && d.getMonth() + 1 === s.month);
		if (match && d > from) out.push(new Date(d));
	}
	return out;
}

export type RetentionUnit = 'days' | 'months' | 'gfs';

/** Calendar months, clamped to month end (31 Jan + 1 month = 28/29 Feb). */
export function expiresAt(unit: 'days' | 'months', value: number, from = new Date()): Date {
	const d = new Date(from);
	if (unit === 'days') d.setDate(d.getDate() + value);
	else {
		const day = d.getDate();
		d.setDate(1);
		d.setMonth(d.getMonth() + value);
		d.setDate(Math.min(day, new Date(d.getFullYear(), d.getMonth() + 1, 0).getDate()));
	}
	return d;
}
