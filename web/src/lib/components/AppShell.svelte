<script lang="ts">
	import type { Snippet } from 'svelte';
	import { afterNavigate } from '$app/navigation';
	import { page } from '$app/state';
	import { t } from '#lib/i18n/index.svelte.ts';
	import Icon from './Icon.svelte';
	import NotificationBell from './NotificationBell.svelte';
	import Sidebar from './Sidebar.svelte';
	import Tour from './Tour.svelte';
	import CommandPalette from './CommandPalette.svelte';
	import { cardMotion } from '#lib/motion.ts';

	let { children }: { children: Snippet } = $props();
	let drawer: HTMLDialogElement;
	let collapsed = $state(false);
	let tour: Tour;
	let palette: CommandPalette;

	function startTour() {
		drawer.close();
		tour.open();
	}

	afterNavigate(() => drawer?.close());

	function toggleMenu() {
		if (matchMedia('(width >= 64rem)').matches) collapsed = !collapsed;
		else drawer.showModal();
	}
</script>

<a href="#main" class="sr-only focus:not-sr-only focus:fixed focus:top-2 focus:left-2 focus:z-50 focus:rounded-lg focus:bg-surface focus:p-2">{t('nav.skip')}</a>

<div class="flex min-h-dvh">
	<aside class="sticky top-0 hidden h-dvh shrink-0 border-r border-line bg-surface lg:block {collapsed ? 'w-16' : 'w-60'}">
		<Sidebar {collapsed} ontour={startTour} />
	</aside>

	<dialog bind:this={drawer} class="drawer" aria-label={t('nav.label')} {...{ closedby: 'any' }}>
		<button type="button" class="btn absolute top-3 right-3 px-2" aria-label={t('action.close')} onclick={() => drawer.close()}><Icon name="close" /></button>
		<Sidebar ontour={startTour} />
	</dialog>

	<div class="min-w-0 flex-1">
		<header class="sticky top-0 z-10 flex items-center gap-2 border-b border-line bg-surface px-4 py-2">
			<button type="button" class="btn px-2" aria-label={t('nav.toggle')} onclick={toggleMenu}><Icon name="menu" /></button>
			<span class="mr-auto"></span>
			<button type="button" class="btn min-h-10 text-dim" aria-keyshortcuts="Meta+K Control+K" onclick={() => palette.open()}>
				<Icon name="search" /><span class="max-sm:sr-only">{t('palette.button')}</span>
				<kbd class="rounded border border-line px-1.5 text-xs max-sm:hidden">{navigator.platform.startsWith('Mac') ? '⌘K' : 'Ctrl K'}</kbd>
			</button>
			<NotificationBell />
		</header>
		<main id="main" class="p-4 lg:p-6" {@attach cardMotion}>
			{@render children()}
		</main>
	</div>
</div>

<Tour bind:this={tour} />
<CommandPalette bind:this={palette} />
