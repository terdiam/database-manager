<script lang="ts">
	import { onMount } from 'svelte';
	import { resolve } from '$app/paths';
	import { dateTime } from '#lib/format.ts';
	import { t } from '#lib/i18n/index.svelte.ts';
	import { markAllRead, notices, simulateArrival } from '#lib/notifications.svelte.ts';
	import Icon from './Icon.svelte';

	const id = $props.id();
	let panel: HTMLElement;
	let announce = $state('');
	const unread = $derived(notices.filter((n) => !n.read).length);

	onMount(() => {
		const timer = setTimeout(() => {
			simulateArrival();
			announce = t('bell.new', { n: unread });
		}, 8000);
		return () => clearTimeout(timer);
	});
</script>

<button type="button" class="btn relative min-h-10 px-2.5" popovertarget="{id}-bell" aria-label={t('bell.label', { n: unread })}>
	<Icon name="bell" size={20} />
	{#if unread}
		<span class="absolute -top-1.5 -right-1.5 grid min-w-5 place-items-center rounded-full bg-bad px-1 text-xs font-bold text-surface tabular-nums" aria-hidden="true">{unread > 9 ? '9+' : unread}</span>
	{/if}
</button>
<span class="sr-only" role="status">{announce}</span>

<div bind:this={panel} id="{id}-bell" popover="auto" class="menu drop w-[min(22rem,calc(100vw-1.5rem))]" role="group" aria-label={t('bell.title')}>
	<div class="flex items-center justify-between gap-2 border-b border-line px-2.5 py-2">
		<h2 class="font-semibold">{t('bell.title')} <span class="rounded-full border border-warn px-1.5 text-xs font-normal text-warn">{t('demo.badge')}</span></h2>
		<button type="button" class="text-xs text-brand underline disabled:text-dim disabled:no-underline" disabled={!unread} onclick={markAllRead}>{t('bell.markRead')}</button>
	</div>
	<ul class="max-h-[min(22rem,60dvh)] overflow-y-auto">
		{#each notices as notice (notice.id)}
			{@const ok = notice.job.status === 'completed'}
			<li>
				<a class="menu-item items-start py-2 {notice.read ? '' : 'bg-muted'}" href={resolve('jobs')} onclick={() => { notice.read = true; panel.hidePopover(); }}>
					<Icon name={ok ? 'check' : 'alert'} class="mt-0.5 {ok ? 'text-ok' : 'text-bad'}" />
					<span class="min-w-0 flex-1">
						<span class="block {notice.read ? '' : 'font-semibold'}">{t(ok ? 'bell.ok' : 'bell.failed', { type: t(`job.${notice.job.type}`), target: notice.job.target })}</span>
						<span class="block text-xs text-dim">{notice.job.id} · {dateTime(notice.job.started)}</span>
					</span>
					{#if !notice.read}<span class="mt-1.5 size-2 rounded-full bg-brand"><span class="sr-only">{t('bell.unread')}</span></span>{/if}
				</a>
			</li>
		{:else}
			<li class="px-2.5 py-6 text-center text-dim">{t('bell.empty')}</li>
		{/each}
	</ul>
	<a class="menu-item justify-center border-t border-line text-brand" href={resolve('jobs')} onclick={() => panel.hidePopover()}>{t('bell.all')}</a>
</div>
