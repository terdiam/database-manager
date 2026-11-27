<script lang="ts">
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { jobs } from '#lib/api/index.ts';
	import { isAdmin } from '#lib/session.svelte.ts';
	import { t } from '#lib/i18n/index.svelte.ts';
	import Icon from './Icon.svelte';
	import { nav } from './nav';
	import logo from '#lib/assets/favicon.svg';
	import UserMenu from './UserMenu.svelte';

	let { collapsed = false, ontour }: { collapsed?: boolean; ontour: () => void } = $props();
	const failed = jobs.filter((job) => job.status === 'failed' && Date.now() - Date.parse(job.started) < 86_400_000).length;
	const active = (href: string) => (href === '/' ? page.url.pathname === '/' : page.url.pathname.startsWith(href));
</script>

<div class="flex h-full flex-col">
<nav class="min-h-0 flex-1 overflow-y-auto p-3" aria-label={t('nav.label')}>
	<p class="mb-3 flex items-center gap-2 px-2 py-1 font-bold">
		<img src={logo} alt="" width="26" height="26" />{#if !collapsed}DB Manager{/if}
	</p>
	{#each nav as section}
		{#if section.group && !collapsed}<p class="mt-4 mb-1 px-2 text-xs font-semibold text-dim uppercase">{t(section.group)}</p>{/if}
		<ul class={section.group && collapsed ? 'mt-3 border-t border-line pt-3' : ''}>
			{#each section.items.filter((i) => !i.admin || isAdmin()) as item}
				<li>
					<a href={resolve(item.href === '/' ? '/(app)' : (item.href.slice(1) as 'jobs'))} aria-current={active(item.href) ? 'page' : undefined} title={collapsed ? t(item.label) : undefined}
						class="flex min-h-9 items-center gap-2.5 rounded-lg px-2 text-sm {active(item.href) ? 'bg-muted font-semibold text-brand' : 'hover:bg-muted'}">
						<Icon name={item.icon} />
						<span class={collapsed ? 'sr-only' : 'flex-1'}>{t(item.label)}</span>
						{#if item.href === '/jobs' && failed && !collapsed}
							<span class="rounded-full bg-muted px-1.5 text-xs font-semibold text-bad" title={t('nav.failed', { n: failed })}>{failed}</span>
						{/if}
					</a>
				</li>
			{/each}
		</ul>
	{/each}
</nav>
<UserMenu {collapsed} {ontour} />
</div>
