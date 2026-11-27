export const presets = ['all', 'today', '7d', '30d', 'date', 'range'] as const;
export type DateValue = { preset: (typeof presets)[number]; date: string; from: string; to: string };

export const emptyDate = (): DateValue => ({ preset: 'all', date: '', from: '', to: '' });

/** Local calendar day as YYYY-MM-DD, the format <input type="date"> uses. */
export const day = (d: Date) =>
	`${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;

/** Inclusive day bounds; an empty string means unbounded. The real API takes the same two parameters. */
export function bounds(v: DateValue, now = new Date()): [from: string, to: string] {
	const back = (days: number) => day(new Date(now.getFullYear(), now.getMonth(), now.getDate() - days));
	switch (v.preset) {
		case 'today': return [back(0), back(0)];
		case '7d': return [back(6), back(0)];
		case '30d': return [back(29), back(0)];
		case 'date': return [v.date, v.date];
		case 'range': return [v.from, v.to];
		default: return ['', ''];
	}
}

export function inDate(iso: string, v: DateValue, now = new Date()): boolean {
	const [from, to] = bounds(v, now);
	const d = day(new Date(iso));
	return (!from || d >= from) && (!to || d <= to);
}
