<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { locale, setLocale, t } from '#lib/i18n/index.svelte.ts';
	import { prefs, setAppearance } from '#lib/prefs.svelte.ts';
	import { isAdmin } from '#lib/session.svelte.ts';
	import Icon, { type IconName } from './Icon.svelte';
	import { nav } from './nav.ts';

	type Command = { label: string; icon: IconName; run: () => void };

	const id = $props.id();
	let dialog: HTMLDialogElement;
	let query = $state('');
	let active = $state(0);

	const go = (path: string) => () => goto(resolve(path === '/' ? '/(app)' : (path.slice(1) as 'jobs')));
	const commands = $derived<Command[]>([
		...nav.flatMap((g) => g.items).filter((i) => !i.admin || isAdmin()).map((i) => ({ label: t(i.label), icon: i.icon, run: go(i.href) })),
		{ label: t('databases.new'), icon: 'plus', run: go('/databases/new') },
		{ label: t('backups.new'), icon: 'plus', run: go('/backups/new') },
		{ label: t('menu.profile'), icon: 'user', run: go('/profile') },
		{ label: `${t('menu.appearance')}: ${t(prefs.appearance === 'dark' ? 'theme.light' : 'theme.dark')}`, icon: 'sun', run: () => setAppearance(prefs.appearance === 'dark' ? 'light' : 'dark') },
		{ label: `${t('lang.label')}: ${locale.current === 'id' ? 'English' : 'Bahasa Indonesia'}`, icon: 'globe', run: () => setLocale(locale.current === 'id' ? 'en' : 'id') }
	]);
	const results = $derived(commands.filter((c) => c.label.toLowerCase().includes(query.trim().toLowerCase())));

	export function open() {
		query = '';
		active = 0;
		if (!dialog.open) dialog.showModal();
	}

	function run(command: Command | undefined) {
		if (!command) return;
		dialog.close();
		command.run();
	}

	function keys(event: KeyboardEvent) {
		if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
			event.preventDefault();
			const step = event.key === 'ArrowDown' ? 1 : -1;
			active = (active + step + results.length) % (results.length || 1);
			document.getElementById(`${id}-${active}`)?.scrollIntoView({ block: 'nearest' });
		} else if (event.key === 'Enter') {
			event.preventDefault();
			run(results[active]);
		}
	}

	function shortcut(event: KeyboardEvent) {
		if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'k') {
			event.preventDefault();
			open();
		}
	}
</script>

<svelte:window onkeydown={shortcut} />

<dialog bind:this={dialog} class="modal mt-[12dvh] p-2" aria-label={t('palette.title')} {...{ closedby: 'any' }}>
	<!-- svelte-ignore a11y_autofocus (the palette exists to be typed into) -->
	<input class="input min-h-11" autofocus role="combobox" aria-expanded="true" aria-controls="{id}-list" aria-activedescendant={results.length ? `${id}-${active}` : undefined}
		aria-label={t('palette.title')} placeholder={t('palette.placeholder')} autocomplete="off" bind:value={query} oninput={() => (active = 0)} onkeydown={keys} />
	<ul id="{id}-list" role="listbox" aria-label={t('palette.title')} class="mt-2 max-h-[50dvh] overflow-y-auto">
		{#each results as command, i (command.label)}
			<!-- svelte-ignore a11y_click_events_have_key_events (keys are handled by the combobox input) -->
			<li id="{id}-{i}" role="option" aria-selected={i === active} class="menu-item cursor-pointer {i === active ? 'bg-muted font-medium' : ''}" onclick={() => run(command)} onpointerenter={() => (active = i)}>
				<Icon name={command.icon} class="text-dim" />{command.label}
			</li>
		{/each}
	</ul>
	{#if !results.length}<p class="p-3 text-center text-sm text-dim" role="status">{t('palette.none')}</p>{/if}
</dialog>
