export const appearances = ['system', 'light', 'dark'] as const;
export const accents = ['indigo', 'emerald', 'rose', 'amber'] as const;
export type Appearance = (typeof appearances)[number];
export type Accent = (typeof accents)[number];

function read(key: string): string | null {
	try {
		return localStorage.getItem(key);
	} catch {
		return null;
	}
}

function store(key: string, value: string | null) {
	try {
		if (value === null) localStorage.removeItem(key);
		else localStorage.setItem(key, value);
	} catch {
		/* storage blocked: the choice applies to this tab only */
	}
}

export const prefs = $state({
	appearance: (read('color-scheme') ?? 'system') as Appearance,
	accent: (read('accent') ?? 'indigo') as Accent
});

export function setAppearance(value: Appearance) {
	prefs.appearance = value;
	document.querySelector<HTMLMetaElement>('meta[name="color-scheme"]')!.content = value === 'system' ? 'light dark' : value;
	store('color-scheme', value === 'system' ? null : value);
}

export function setAccent(value: Accent) {
	prefs.accent = value;
	document.documentElement.dataset.accent = value;
	store('accent', value);
}
