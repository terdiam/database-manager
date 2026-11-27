import { messages, type Key } from './messages';

export type Locale = 'id' | 'en';

function stored(): Locale {
	try {
		return localStorage.getItem('locale') === 'en' ? 'en' : 'id';
	} catch {
		return 'id';
	}
}

export const locale = $state({ current: stored() });

export function setLocale(next: Locale) {
	locale.current = next;
	try {
		localStorage.setItem('locale', next);
	} catch {
		/* private mode: choice lasts for this tab only */
	}
}

export function t(key: Key, vars: Record<string, string | number> = {}): string {
	const text = messages[key][locale.current === 'id' ? 0 : 1];
	return text.replace(/\{(\w+)\}/g, (_, name) => String(vars[name] ?? ''));
}

export type { Key };
