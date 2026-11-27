<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { request } from '#lib/api/http.ts';
	import { errorText } from '#lib/errors.ts';
	import { locale, setLocale, t } from '#lib/i18n/index.svelte.ts';
	import { accents, appearances, prefs, setAccent, setAppearance } from '#lib/prefs.svelte.ts';
	import { session } from '#lib/session.svelte.ts';
	import { toast } from './toast.svelte.ts';
	import Avatar from './Avatar.svelte';
	import Icon from './Icon.svelte';

	let { collapsed = false, ontour }: { collapsed?: boolean; ontour: () => void } = $props();
	const id = $props.id();
	let menu: HTMLElement;

	const close = () => menu.hidePopover();
	const currentUser = $derived(session.user ?? { name: '', email: '', role: '' });

	async function logout() {
		close();
		try {
			await request('POST', '/auth/logout');
		} catch (e) {
			return toast(errorText(e));
		}
		session.user = null;
		await goto(resolve('login'), { invalidateAll: true });
	}
	const swatch = { indigo: '#3b4fd8', emerald: '#0f7a55', rose: '#b8285a', amber: '#8f5200' };
	const languages = { id: 'Bahasa Indonesia', en: 'English' } as const;
</script>

{#snippet submenu(key: string, label: string, icon: 'palette' | 'sun' | 'globe', body: import('svelte').Snippet)}
	<button type="button" class="menu-item" popovertarget="{id}-{key}">
		<Icon name={icon} class="text-dim" /><span class="flex-1">{label}</span><Icon name="chevron" size={16} />
	</button>
	<div id="{id}-{key}" popover="auto" class="menu submenu" role="group" aria-label={label}>{@render body()}</div>
{/snippet}

{#snippet choice(selected: boolean, label: string, onclick: () => void, color?: string)}
	<button type="button" class="menu-item" aria-pressed={selected} {onclick}>
		{#if color}<span class="size-3.5 rounded-full" style:background={color} aria-hidden="true"></span>{/if}
		<span class="flex-1">{label}</span>
		{#if selected}<Icon name="check" size={16} class="text-brand" />{/if}
	</button>
{/snippet}

{#snippet themeBody()}
	{#each accents as accent}{@render choice(prefs.accent === accent, t(`accent.${accent}`), () => setAccent(accent), swatch[accent])}{/each}
{/snippet}
{#snippet appearanceBody()}
	{#each appearances as value}{@render choice(prefs.appearance === value, t(`theme.${value}`), () => setAppearance(value))}{/each}
{/snippet}
{#snippet languageBody()}
	{#each ['id', 'en'] as const as value}{@render choice(locale.current === value, languages[value], () => setLocale(value))}{/each}
{/snippet}

<div class="border-t border-line p-2">
	<button type="button" class="flex w-full items-center gap-2.5 rounded-lg p-2 text-left text-sm hover:bg-muted" popovertarget="{id}-menu" aria-label={t('menu.account', { name: currentUser.name })}>
		<Avatar name={currentUser.name} />
		{#if !collapsed}
			<span class="min-w-0 flex-1">
				<span class="block truncate font-medium">{currentUser.name}</span>
				<span class="block truncate text-xs text-dim capitalize">{currentUser.role}</span>
			</span>
			<Icon name="updown" size={16} class="text-dim" />
		{/if}
	</button>

	<div bind:this={menu} id="{id}-menu" popover="auto" class="menu">
		<div class="mb-1 flex items-center gap-2.5 border-b border-line px-2.5 py-2">
			<Avatar name={currentUser.name} />
			<span class="min-w-0">
				<span class="block truncate font-semibold">{currentUser.name}</span>
				<span class="block truncate text-xs text-dim">{currentUser.email}</span>
			</span>
		</div>
		<a class="menu-item" href={resolve('profile')} onclick={close}><Icon name="user" class="text-dim" />{t('menu.profile')}</a>
		{#if currentUser.role === 'admin'}<a class="menu-item" href={resolve('settings')} onclick={close}><Icon name="settings" class="text-dim" />{t('nav.settings')}</a>{/if}
		<hr class="my-1 border-line" />
		<a class="menu-item" href={resolve('help')} onclick={close}><Icon name="book" class="text-dim" />{t('menu.docs')}</a>
		<button type="button" class="menu-item" onclick={() => { close(); ontour(); }}><Icon name="map" class="text-dim" />{t('menu.tour')}</button>
		<hr class="my-1 border-line" />
		{@render submenu('theme', t('menu.theme'), 'palette', themeBody)}
		{@render submenu('appearance', t('menu.appearance'), 'sun', appearanceBody)}
		{@render submenu('language', t('lang.label'), 'globe', languageBody)}
		<hr class="my-1 border-line" />
		<button type="button" class="menu-item" onclick={logout}><Icon name="logout" class="text-dim" />{t('action.logout')}</button>
	</div>
</div>
