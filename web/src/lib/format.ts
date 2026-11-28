import { locale } from './i18n/index.svelte';

export function bytes(n: number): string {
	const units = ['B', 'KB', 'MB', 'GB', 'TB'];
	const i = n > 0 ? Math.min(Math.floor(Math.log(n) / Math.log(1024)), units.length - 1) : 0;
	return `${(n / 1024 ** i).toFixed(i ? 1 : 0)} ${units[i]}`;
}

export function dateTime(iso: string): string {
	return new Intl.DateTimeFormat(locale.current, { dateStyle: 'medium', timeStyle: 'short' }).format(new Date(iso));
}

/** With seconds: events in one minute have to be told apart. */
export function exactTime(iso: string): string {
	return new Intl.DateTimeFormat(locale.current, { dateStyle: 'medium', timeStyle: 'medium' }).format(new Date(iso));
}

export function date(value: string | Date): string {
	return new Intl.DateTimeFormat(locale.current, { dateStyle: 'medium' }).format(new Date(value));
}
