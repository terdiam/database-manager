<script lang="ts">
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { asset, resolve } from '$app/paths';
	import { e2eeInfo, request, type AuthState } from '#lib/api/http.ts';
	import Icon from '#lib/components/Icon.svelte';
	import LoginShowcase from '#lib/components/LoginShowcase.svelte';
	import NewPassword from '#lib/components/NewPassword.svelte';
	import PasswordInput from '#lib/components/PasswordInput.svelte';
	import Turnstile from '#lib/components/Turnstile.svelte';
	import { errorText } from '#lib/errors.ts';
	import { locale, setLocale, t } from '#lib/i18n/index.svelte.ts';
	import { enter, shake } from '#lib/motion.ts';
	import { prefs, setAppearance } from '#lib/prefs.svelte.ts';

	let setup = $state(false);
	let name = $state('');
	let email = $state('');
	let password = $state('');
	let error = $state('');
	let busy = $state(false);
	let done = $state(false);
	let capsLock = $state(false);
	let server = $state<'checking' | 'online' | 'offline'>('checking');
	let card: HTMLFormElement;
	let siteKey = $state('');
	let encrypted = $state(false);
	let token = $state('');
	let widget = $state<Turnstile>();

	const hour = new Date().getHours();
	const greeting = hour < 11 ? 'morning' : hour < 15 ? 'noon' : hour < 18 ? 'afternoon' : 'evening';
	const dark = $derived(prefs.appearance === 'dark' || (prefs.appearance === 'system' && matchMedia('(prefers-color-scheme: dark)').matches));

	async function check() {
		server = 'checking';
		try {
			const state = await request<AuthState>('GET', '/auth/state');
			server = 'online';
			error = '';
			encrypted = (await e2eeInfo()).required;
			if (state.user) await goto(resolve('/(app)'));
			setup = state.setup_required;
			siteKey = state.turnstile_site_key ?? '';
		} catch (e) {
			server = 'offline';
			error = errorText(e);
		}
	}
	onMount(check);

	const keys = (event: Event) => {
		if (event instanceof KeyboardEvent) capsLock = event.getModifierState?.('CapsLock') ?? false;
	};

	async function submit(event: SubmitEvent) {
		event.preventDefault();
		busy = true;
		error = '';
		try {
			const credentials = setup ? { name, email, password } : { email, password };
			await request('POST', setup ? '/auth/setup' : '/auth/login', { ...credentials, turnstile_token: token || undefined });
			done = true;
			await new Promise((r) => setTimeout(r, 500)); // let the success state be seen
			await goto(resolve('/(app)'), { invalidateAll: true });
		} catch (e) {
			error = errorText(e);
			widget?.reset();
			shake(card);
		} finally {
			busy = false;
		}
	}

	const logo = asset('favicon.svg');
</script>

<svelte:head><title>{t(setup ? 'login.setup' : 'login.title')} · DB Manager</title></svelte:head>

<div class="grid min-h-dvh lg:grid-cols-[1.1fr_1fr]">
	<!-- Form first in the DOM: it is what people came for, and what a phone shows first. -->
	<main class="flex flex-col p-4 lg:order-2">
		<div class="flex flex-wrap items-center justify-end gap-2">
			<span class="mr-auto inline-flex items-center gap-2 text-xs text-dim" role="status">
				<span class="size-2 rounded-full {server === 'online' ? 'bg-green-500' : server === 'offline' ? 'bg-red-500' : 'bg-amber-500 motion-safe:animate-pulse'}" aria-hidden="true"></span>
				{t(`login.server.${server}`)}
			</span>
			{#if encrypted}
				<span class="inline-flex items-center gap-1 rounded-full border border-line px-2 py-0.5 text-xs text-ok" title={t('login.encryptedHint')}>
					<Icon name="shield" size={13} /><span class="max-sm:sr-only">{t('login.encrypted')}</span>
				</span>
			{/if}
			{#if server === 'offline'}<button type="button" class="btn min-h-9 text-xs" onclick={check}>{t('action.retry')}</button>{/if}
			<button type="button" class="btn min-h-9 px-2.5" aria-label={t(dark ? 'theme.light' : 'theme.dark')} title={t(dark ? 'theme.light' : 'theme.dark')} onclick={() => setAppearance(dark ? 'light' : 'dark')}>
				<Icon name={dark ? 'sun' : 'moon'} />
			</button>
			<button type="button" class="btn min-h-9" onclick={() => setLocale(locale.current === 'id' ? 'en' : 'id')}>
				<Icon name="globe" />{locale.current === 'id' ? 'English' : 'Indonesia'}
			</button>
		</div>

		<div class="grid flex-1 place-items-center py-6">
			<!-- svelte-ignore a11y_no_noninteractive_element_interactions (only reads Caps Lock state from keys typed in the fields) -->
			<form bind:this={card} class="card w-full max-w-md p-6 shadow-xl sm:p-8" onsubmit={submit} onkeydown={keys} onkeyup={keys} {@attach enter}>
				<img src={logo} alt="" width="48" height="48" class="mb-4" data-enter />
				<p class="text-sm font-medium text-brand" data-enter>{t(`login.greeting.${greeting}`)}</p>
				<h1 class="text-2xl font-bold tracking-tight">{t(setup ? 'login.setup' : 'login.welcome')}</h1>
				<p class="mt-1 mb-5 text-sm text-dim" data-enter>{t(setup ? 'login.setupHint' : 'login.hint')}</p>

				<div data-enter>
					{#if setup}
						<label class="mb-3 block text-sm font-medium">{t('col.name')}<input class="input mt-1 min-h-11" name="name" autocomplete="name" maxlength="100" required bind:value={name} /></label>
					{/if}
					<label class="mb-3 block text-sm font-medium">Email<input class="input mt-1 min-h-11" type="email" name="email" autocomplete="username" required bind:value={email} /></label>
					{#if setup}
						<NewPassword label="Password" bind:value={password} />
					{:else}
						<label for="password" class="mb-1 block text-sm font-medium">Password</label>
						<PasswordInput id="password" name="password" autocomplete="current-password" required bind:value={password} />
					{/if}
					<p class="mt-1 flex min-h-5 items-center gap-1.5 text-xs text-warn" role="status">
						{#if capsLock}<Icon name="alert" size={14} />{t('login.capsLock')}{/if}
					</p>
				</div>

				{#if siteKey}<Turnstile bind:this={widget} {siteKey} action={setup ? 'setup' : 'login'} bind:token />{/if}

				<p class="mb-3 flex items-center gap-2 rounded-lg border border-bad p-3 text-sm text-bad empty:hidden" role="alert">{error}</p>

				<div data-enter>
					<button type="submit" class="btn btn-primary min-h-11 w-full text-base" disabled={busy || server === 'offline' || (!!siteKey && !token)}>
						{#if done}
							<Icon name="check" />{t('login.success')}
						{:else if busy}
							<Icon name="spin" class="motion-safe:animate-spin" />{t('login.checking')}
						{:else}
							{t(setup ? 'login.create' : 'login.title')}<Icon name="chevron" size={16} />
						{/if}
					</button>
					{#if !setup}<p class="mt-4 text-center text-xs text-dim">{t('login.forgot')}</p>{/if}
				</div>
			</form>
		</div>
	</main>

	<aside class="showcase grid place-items-center border-line p-6 max-lg:border-t lg:order-1 lg:border-r lg:p-12">
		<LoginShowcase />
	</aside>
</div>
