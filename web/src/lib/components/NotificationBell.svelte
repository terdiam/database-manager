<script lang="ts">
	import { onMount } from 'svelte';
	import { resolve } from '$app/paths';
	import { live } from '#lib/events.ts';
	import { dateTime } from '#lib/format.ts';
	import { t } from '#lib/i18n/index.svelte.ts';
	import { isUnread, markAllRead, notices, refresh } from '#lib/notifications.svelte.ts';
	import Icon from './Icon.svelte';

	const id = $props.id();
	let panel: HTMLElement;
	const unread = $derived(notices.jobs.filter(isUnread).length);

	// `users` also changes when this account marks them read elsewhere.
	onMount(() => live(['jobs', 'users'], refresh, { now: true }));
</script>

<button type="button" class="btn relative min-h-10 px-2.5" popovertarget="{id}-bell" aria-label={t('bell.label', { n: unread })}>
	<!-- A new key restarts the animations for every arrival. -->
	{#key notices.arrived}
		<span class="bell {unread ? 'bell-unread' : ''} {notices.arrived ? 'bell-ring' : ''}"><Icon name="bell" size={20} /></span>
		{#if unread}
			<span class="absolute -top-1.5 -right-1.5 grid min-w-5 place-items-center rounded-full bg-bad px-1 text-xs font-bold text-surface tabular-nums {notices.arrived ? 'bell-badge' : ''}" aria-hidden="true">{unread > 9 ? '9+' : unread}</span>
		{/if}
	{/key}
</button>
<span class="sr-only" role="status">{unread ? t('bell.new', { n: unread }) : ''}</span>

<div bind:this={panel} id="{id}-bell" popover="auto" class="menu drop w-[min(22rem,calc(100vw-1.5rem))]" role="group" aria-label={t('bell.title')}>
	<div class="flex items-center justify-between gap-2 border-b border-line px-2.5 py-2">
		<h2 class="font-semibold">{t('bell.title')}</h2>
		<button type="button" class="text-xs text-brand underline disabled:text-dim disabled:no-underline" disabled={!unread} onclick={markAllRead}>{t('bell.markRead')}</button>
	</div>
	<ul class="max-h-[min(22rem,60dvh)] overflow-y-auto">
		{#each notices.jobs as job (job.id)}
			{@const ok = job.status === 'completed'}
			<li>
				<a class="menu-item items-start py-2 {isUnread(job) ? 'bg-muted' : ''}" href={resolve('jobs')} onclick={() => panel.hidePopover()}>
					<Icon name={ok ? 'check' : 'alert'} class="mt-0.5 {ok ? 'text-ok' : 'text-bad'}" />
					<span class="min-w-0 flex-1">
						<span class="block {isUnread(job) ? 'font-semibold' : ''}">{t(ok ? 'bell.ok' : 'bell.failed', { type: t(`job.${job.kind}`), target: job.target })}</span>
						<span class="block text-xs text-dim">{dateTime(job.finished_at ?? job.created_at)}</span>
					</span>
					{#if isUnread(job)}<span class="mt-1.5 size-2 rounded-full bg-brand"><span class="sr-only">{t('bell.unread')}</span></span>{/if}
				</a>
			</li>
		{:else}
			<li class="px-2.5 py-6 text-center text-dim">{t('bell.empty')}</li>
		{/each}
	</ul>
	<a class="menu-item justify-center border-t border-line text-brand" href={resolve('jobs')} onclick={() => panel.hidePopover()}>{t('bell.all')}</a>
</div>
