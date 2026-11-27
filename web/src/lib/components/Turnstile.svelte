<script lang="ts" module>
	type Api = {
		render: (element: HTMLElement, options: Record<string, unknown>) => string;
		reset: (id: string) => void;
		remove: (id: string) => void;
	};
	declare global {
		interface Window {
			turnstile?: Api;
		}
	}

	// Cloudflare's script is only fetched when the server says Turnstile is on.
	let loading: Promise<Api> | undefined;
	function load() {
		return (loading ??= new Promise<Api>((resolve, reject) => {
			const script = document.createElement('script');
			script.src = 'https://challenges.cloudflare.com/turnstile/v0/api.js?render=explicit';
			script.async = true;
			script.onload = () => resolve(window.turnstile!);
			script.onerror = () => {
				loading = undefined; // allow a retry
				script.remove();
				reject(new Error('turnstile script'));
			};
			document.head.append(script);
		}));
	}
</script>

<script lang="ts">
	import { locale, t } from '#lib/i18n/index.svelte.ts';
	import { prefs } from '#lib/prefs.svelte.ts';

	/** `token` is empty until the check passes and again after it expires or is used. */
	let { siteKey, action, token = $bindable('') }: { siteKey: string; action: 'login' | 'setup'; token?: string } = $props();
	let failed = $state(false);
	let api: Api | undefined;
	let widget = '';

	/** Tokens are single use: call after every submit. */
	export function reset() {
		token = '';
		if (widget) api?.reset(widget);
	}

	function mount(element: HTMLElement) {
		// Read here so the widget is rebuilt when action, theme or language changes.
		const expected = action; // the server rejects a token issued for any other action
		const theme = prefs.appearance === 'system' ? 'auto' : prefs.appearance;
		const language = locale.current;
		let cancelled = false;
		load().then(
			(loaded) => {
				if (cancelled) return;
				api = loaded;
				failed = false;
				widget = loaded.render(element, {
					sitekey: siteKey,
					action: expected,
					theme,
					language,
					size: 'flexible',
					callback: (value: string) => (token = value),
					'expired-callback': () => (token = ''),
					'error-callback': () => {
						token = '';
						failed = true;
					}
				});
			},
			() => (failed = true)
		);
		return () => {
			cancelled = true;
			if (widget) api?.remove(widget);
			widget = '';
			token = '';
		};
	}
</script>

<div class="mb-3">
	<div class="min-h-[65px]" {@attach mount}></div>
	{#if failed}<p class="mt-1 text-sm text-bad" role="alert">{t('login.captchaFailed')}</p>{/if}
</div>
