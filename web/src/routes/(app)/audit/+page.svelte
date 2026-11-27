<script lang="ts">
	import { onMount } from 'svelte';
	import { request, type AuditEntry, type AuditPage, type User } from '#lib/api/http.ts';
	import DataTable from '#lib/components/DataTable.svelte';
	import DateFilter from '#lib/components/DateFilter.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import Select from '#lib/components/Select.svelte';
	import { bounds, emptyDate } from '#lib/daterange.ts';
	import { errorText } from '#lib/errors.ts';
	import { dateTime } from '#lib/format.ts';
	import { t, type Key } from '#lib/i18n/index.svelte.ts';
	import { messages } from '#lib/i18n/messages.ts';
	import { live } from '#lib/events.ts';

	const actions = ['login', 'backup', 'restore', 'create', 'update', 'delete', 'download'] as const;
	const label = (action: string) => (`audit.${action}` in messages ? t(`audit.${action}` as Key) : action);

	let date = $state(emptyDate());
	let user = $state('');
	let action = $state('');
	let query = $state('');
	let users = $state<User[]>([]);
	let rows = $state<AuditEntry[]>([]);
	let next = $state<number | null>(null);
	let error = $state('');
	let sequence = 0;
	let paged = false; // older pages are on screen; a refresh would drop them

	request<User[]>('GET', '/users').then((list) => (users = list), (e) => (error = errorText(e)));

	/** Local midnight of the chosen day, as the instant the server compares against. */
	const instant = (day: string, plusDays = 0) => {
		const d = new Date(`${day}T00:00:00`);
		d.setDate(d.getDate() + plusDays);
		return d.toISOString();
	};

	const params = $derived.by(() => {
		const [from, to] = bounds(date);
		const p = new URLSearchParams();
		if (from) p.set('from', instant(from));
		if (to) p.set('to', instant(to, 1));
		if (user) p.set('user', user);
		if (action) p.set('action', action);
		if (query) p.set('q', query);
		return p.toString();
	});

	async function load(filter: string, before: number | null) {
		const mine = ++sequence;
		paged = before !== null;
		try {
			const page = await request<AuditPage>('GET', `/audit?${filter}${before ? `&before=${before}` : ''}`);
			if (mine !== sequence) return; // a newer filter already replaced this one
			rows = before ? [...rows, ...page.items] : page.items;
			next = page.next;
			error = '';
		} catch (e) {
			error = errorText(e);
		}
	}

	// Short delay so typing in the search box does not send a request per key.
	$effect(() => {
		const filter = params;
		const timer = setTimeout(() => load(filter, null), 250);
		return () => clearTimeout(timer);
	});
	onMount(() => live(['audit_logs'], () => paged || load(params, null)));
</script>

<PageHeader title={t('nav.audit')} description={t('audit.desc')} />

<search class="mb-3 flex flex-wrap items-center gap-2">
	<DateFilter bind:value={date} />
	<Select class="w-52" aria-label={t('audit.user')} bind:value={user} options={[{ value: '', label: t('audit.allUsers'), icon: 'users' as const }, ...users.map((u) => ({ value: u.id, label: u.name, description: u.email }))]} />
	<Select class="w-56" aria-label={t('audit.action')} bind:value={action} options={[{ value: '', label: t('audit.allActions'), icon: 'list' as const }, ...actions.map((a) => ({ value: a, label: t(`audit.${a}`) }))]} />
	<input class="input min-h-11 w-full sm:w-56" type="search" aria-label={t('audit.search')} placeholder={t('audit.search')} bind:value={query} />
</search>

<p class="mb-2 text-sm text-bad empty:hidden" role="alert">{error}</p>
<p class="mb-2 text-xs text-dim" role="status">{t('audit.shown', { n: rows.length })}</p>

<DataTable label={t('nav.audit')} columns={[t('audit.time'), t('audit.user'), t('audit.action'), t('col.target'), 'IP', t('audit.result')]} rows={rows.map((r) => ({ ...r, id: String(r.id) }))}>
	{#snippet cell(log, i)}
		{#if i === 0}<span class="whitespace-nowrap">{dateTime(log.created_at)}</span>
		{:else if i === 1}{log.user_name}
		{:else if i === 2}{label(log.action)}
		{:else if i === 3}{log.target}
		{:else if i === 4}<code class="text-xs">{log.ip}</code>
		{:else}<span class="inline-flex rounded-full bg-muted px-2 py-0.5 text-xs font-medium {log.ok ? 'text-ok' : 'text-bad'}">{log.ok ? '✓' : '✕'} {t(log.ok ? 'status.completed' : 'status.failed')}</span>{/if}
	{/snippet}
</DataTable>

{#if next}
	<button type="button" class="btn mt-3" onclick={() => load(params, next)}>{t('action.more')}</button>
{/if}
