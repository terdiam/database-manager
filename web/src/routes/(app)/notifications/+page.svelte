<script lang="ts">
	import IconButton from '#lib/components/IconButton.svelte';
	import { onMount } from 'svelte';
	import { request, type Channel } from '#lib/api/http.ts';
	import BrandIcon from '#lib/components/BrandIcon.svelte';
	import ConfirmDialog from '#lib/components/ConfirmDialog.svelte';
	import DataTable from '#lib/components/DataTable.svelte';
	import FormField from '#lib/components/FormField.svelte';
	import Modal from '#lib/components/Modal.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import PasswordInput from '#lib/components/PasswordInput.svelte';
	import Select from '#lib/components/Select.svelte';
	import { toast } from '#lib/components/toast.svelte.ts';
	import { errorText } from '#lib/errors.ts';
	import { live } from '#lib/events.ts';
	import { dateTime } from '#lib/format.ts';
	import { t } from '#lib/i18n/index.svelte.ts';
	import { isAdmin } from '#lib/session.svelte.ts';

	const events = ['backup_failed', 'restore_failed', 'agent_offline'] as const;
	const names = { telegram: 'Telegram', mattermost: 'Mattermost' };
	const blank = () => ({ name: '', kind: 'telegram' as string, bot_token: '', chat_id: '', webhook_url: '', events: [...events] as string[] });

	let channels = $state<Channel[]>([]);
	let error = $state('');
	let loadError = $state('');
	let formError = $state('');
	let form = $state(blank());
	let removing = $state<Channel>();
	let editing = $state<Channel>();
	// The stored token or URL is never sent back, so changing it means typing it again.
	let replace = $state(true);
	let modal: Modal;
	let confirm: ConfirmDialog;

	async function load() {
		if (!isAdmin()) return;
		try {
			channels = await request<Channel[]>('GET', '/notification-channels');
			loadError = '';
		} catch (e) {
			loadError = errorText(e);
		}
	}
	onMount(() => {
		load();
		return live(['notification_channels'], load);
	});

	function open(channel?: Channel) {
		editing = channel;
		replace = !channel;
		form = channel ? { ...blank(), name: channel.name, kind: channel.kind, events: [...channel.events] } : blank();
		formError = '';
		modal.open();
	}

	async function create(close: () => void) {
		const { name, kind, bot_token, chat_id, webhook_url } = form;
		if (!form.events.length) {
			formError = t('error.events');
			return;
		}
		const secret = kind === 'telegram' ? { kind, bot_token: bot_token.trim(), chat_id: chat_id.trim() } : { kind, webhook_url: webhook_url.trim() };
		try {
			if (editing) await request('PUT', `/notification-channels/${editing.id}`, { name, events: form.events, secret: replace ? secret : undefined });
			else await request('POST', '/notification-channels', { name, events: form.events, secret });
			close();
			form = blank();
			toast(t(editing ? 'toast.updated' : 'toast.created', { name }));
			await load();
		} catch (e) {
			formError = errorText(e);
		}
	}

	async function test(channel: Channel) {
		try {
			const result = await request<{ error: string | null }>('POST', `/notification-channels/${channel.id}/test`);
			error = result.error ? t('notifications.testFailed', { name: channel.name, error: result.error }) : '';
			if (!result.error) toast(t('toast.test', { name: channel.name }));
			await load();
		} catch (e) {
			error = errorText(e);
		}
	}

	async function remove() {
		try {
			await request('DELETE', `/notification-channels/${removing!.id}`);
			toast(t('toast.deleted', { name: removing!.name }));
			error = '';
			await load();
		} catch (e) {
			error = errorText(e);
		}
	}
</script>

<PageHeader title={t('nav.notifications')} description={t('notifications.desc')}>
	{#snippet actions()}{#if isAdmin()}<button type="button" class="btn btn-primary" onclick={() => open()}>{t('notifications.new')}</button>{/if}{/snippet}
</PageHeader>

{#if !isAdmin()}
	<p class="card text-sm">{t('error.forbidden')}</p>
{:else}
	<p class="mb-3 text-sm text-bad empty:hidden" role="alert">{error || loadError}</p>

	<DataTable label={t('nav.notifications')} columns={[t('col.name'), t('col.type'), t('col.target'), t('col.events'), t('col.status'), t('col.actions')]} rows={channels}>
		{#snippet cell(c, i)}
			{#if i === 0}<strong>{c.name}</strong>
			{:else if i === 1}<span class="inline-flex items-center gap-2"><BrandIcon name={names[c.kind]} />{names[c.kind]}</span>
			{:else if i === 2}<code class="text-xs break-all">{c.target}</code>
			{:else if i === 3}{c.events.map((e) => t(`event.${e as (typeof events)[number]}`)).join(', ')}
			{:else if i === 4}
				{#if c.last_error}<span class="text-bad">✕ {c.last_error}</span>
				{:else if c.last_sent_at}<span class="text-ok">✓ {dateTime(c.last_sent_at)}</span>
				{:else}<span class="text-dim">{t('notifications.never')}</span>{/if}
			{:else}<span class="flex flex-wrap justify-end gap-1">
				<IconButton icon="send" label={t('notifications.test')} onclick={() => test(c)} />
				<IconButton icon="edit" label={t('action.edit')} onclick={() => open(c)} />
				<IconButton icon="trash" label={t('action.delete')} onclick={() => { removing = c; confirm.open(); }} danger />
			</span>{/if}
		{/snippet}
	</DataTable>
{/if}

<Modal bind:this={modal} title={editing ? `${t('action.edit')}: ${editing.name}` : t('notifications.new')}>
	{#snippet children(close)}
		<form onsubmit={(e) => { e.preventDefault(); create(close); }}>
			<FormField label={t('col.name')}>{#snippet children(a)}<input {...a} class="input" maxlength="64" required bind:value={form.name} />{/snippet}</FormField>
			{#if editing}<label class="mb-3 flex items-center gap-2 text-sm font-medium"><input type="checkbox" bind:checked={replace} />{t('field.replaceSecret')}</label>{/if}
			{#if replace}
			<FormField label={t('col.type')}>{#snippet children(a)}<Select {...a} bind:value={form.kind} options={Object.entries(names).map(([value, label]) => ({ value, label, brand: label }))} />{/snippet}</FormField>
			{#if form.kind === 'telegram'}
				<FormField label="Bot token" help={t('help.secret')}>{#snippet children(a)}<PasswordInput {...a} autocomplete="new-password" required bind:value={form.bot_token} />{/snippet}</FormField>
				<FormField label="Chat ID" help={t('help.chatId')}>{#snippet children(a)}<input {...a} class="input" maxlength="64" autocomplete="off" required bind:value={form.chat_id} />{/snippet}</FormField>
			{:else}
				<FormField label="Incoming webhook URL" help={t('help.secret')}>{#snippet children(a)}<PasswordInput {...a} autocomplete="new-password" required bind:value={form.webhook_url} />{/snippet}</FormField>
			{/if}
			{/if}
			<fieldset class="mb-3 text-sm">
				<legend class="mb-1 font-medium">{t('col.events')}</legend>
				{#each events as event}
					<label class="flex items-center gap-2 py-1"><input type="checkbox" value={event} bind:group={form.events} />{t(`event.${event}`)}</label>
				{/each}
			</fieldset>
			<p class="text-sm text-bad empty:hidden" role="alert">{formError}</p>
			<div class="mt-4 flex justify-end gap-2">
				<button type="button" class="btn" onclick={close}>{t('action.cancel')}</button>
				<button type="submit" class="btn btn-primary">{t('action.save')}</button>
			</div>
		</form>
	{/snippet}
</Modal>

<ConfirmDialog bind:this={confirm} title={t('action.delete')} message={t('confirm.delete', { name: removing?.name ?? '' })} confirmLabel={t('action.delete')} onconfirm={remove} />
