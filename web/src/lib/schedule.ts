export type Schedule = {
	kind: 'manual' | 'hours' | 'daily' | 'weekly' | 'monthly' | 'yearly' | 'cron';
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

/** The five-field pattern the server runs; `null` means only by hand. */
export function toCron(s: Schedule): string | null {
	const [hour, minute] = s.time.split(':').map(Number);
	const at = `${minute || 0} ${hour || 0}`;
	switch (s.kind) {
		case 'manual':
			return null;
		case 'hours':
			return `0 */${s.every} * * *`;
		case 'daily':
			return `${at} * * *`;
		case 'weekly':
			return `${at} * * ${s.weekday}`;
		case 'monthly':
			return `${at} ${s.day} * *`;
		case 'yearly':
			return `${at} ${s.day} ${s.month} *`;
		case 'cron':
			return s.cron.trim();
	}
}

/** The form for a stored pattern; anything the simple choices cannot say stays cron. */
export function fromCron(pattern: string | null): Schedule {
	if (!pattern) return newSchedule('manual');
	const every = /^0 \*\/(\d+) \* \* \*$/.exec(pattern);
	if (every) return newSchedule('hours', { every: Number(every[1]), cron: pattern });
	const match = /^(\d+) (\d+) (\*|\d+) (\*|\d+) (\*|\d+)$/.exec(pattern);
	if (match) {
		const [, minute, hour, day, month, weekday] = match;
		const time = `${hour.padStart(2, '0')}:${minute.padStart(2, '0')}`;
		const shape = [day, month, weekday].map((f) => (f === '*' ? '*' : 'n')).join('');
		const base = { time, cron: pattern };
		if (shape === '***') return newSchedule('daily', base);
		if (shape === '**n') return newSchedule('weekly', { ...base, weekday: Number(weekday) % 7 });
		if (shape === 'n**') return newSchedule('monthly', { ...base, day: Number(day) });
		if (shape === 'nn*') return newSchedule('yearly', { ...base, day: Number(day), month: Number(month) });
	}
	return newSchedule('cron', { cron: pattern });
}

export type RetentionUnit = 'all' | 'days' | 'months' | 'gfs';
export type Retention = { unit: RetentionUnit; amount: number; daily: number; weekly: number; monthly: number };

export const newRetention = (patch: Partial<Retention> = {}): Retention => ({ unit: 'days', amount: 30, daily: 7, weekly: 4, monthly: 12, ...patch });

/** What the API stores for a schedule and retention. */
export type Plan = {
	schedule: string | null;
	timezone: string;
	retention_unit: Exclude<RetentionUnit, 'all'> | null;
	retention_value: number | null;
	keep_daily: number | null;
	keep_weekly: number | null;
	keep_monthly: number | null;
};

export function toPlan(schedule: Schedule, timezone: string, r: Retention): Plan {
	const timed = r.unit === 'days' || r.unit === 'months';
	const gfs = r.unit === 'gfs';
	return {
		schedule: toCron(schedule),
		timezone,
		retention_unit: r.unit === 'all' ? null : r.unit,
		retention_value: timed ? r.amount : null,
		keep_daily: gfs ? r.daily : null,
		keep_weekly: gfs ? r.weekly : null,
		keep_monthly: gfs ? r.monthly : null
	};
}

export function retentionOf(plan: Plan): Retention {
	return newRetention({
		unit: plan.retention_unit ?? 'all',
		amount: plan.retention_value ?? 30,
		daily: plan.keep_daily ?? 7,
		weekly: plan.keep_weekly ?? 4,
		monthly: plan.keep_monthly ?? 12
	});
}

export const localZone = () => Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC';

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
