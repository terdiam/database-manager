<script lang="ts">
	import IconButton from '#lib/components/IconButton.svelte';
	import { onMount } from 'svelte';
	import { invalidateAll } from '$app/navigation';
	import { request, type Role, type User } from '#lib/api/http.ts';
	import ConfirmDialog from '#lib/components/ConfirmDialog.svelte';
	import DataTable from '#lib/components/DataTable.svelte';
	import FormField from '#lib/components/FormField.svelte';
	import Modal from '#lib/components/Modal.svelte';
	import NewPassword from '#lib/components/NewPassword.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import Select from '#lib/components/Select.svelte';
	import { toast } from '#lib/components/toast.svelte.ts';
	import { errorText } from '#lib/errors.ts';
	import { t } from '#lib/i18n/index.svelte.ts';
	import { live } from '#lib/events.ts';
	import { session } from '#lib/session.svelte.ts';

	const blank = () => ({ name: '', email: '', password: '', role: 'viewer' as string });
	let users = $state<User[]>([]);
	let loadError = $state('');
	let actionError = $state('');
	let form = $state(blank());
	let formError = $state('');
	let removing = $state<User>();
	let editing = $state<User>();
	let modal: Modal;
	let confirm: ConfirmDialog;

	async function load() {
		try {
			users = await request<User[]>('GET', '/users');
			loadError = '';
		} catch (e) {
			loadError = errorText(e);
		}
	}
	onMount(() => {
		load();
		return live(['users'], load);
	});

	function open(user?: User) {
		editing = user;
		form = user ? { ...blank(), name: user.name, email: user.email, role: user.role } : blank();
		formError = '';
		modal.open();
	}

	async function create(event: SubmitEvent & { currentTarget: HTMLFormElement }, close: () => void) {
		event.preventDefault();
		const element = event.currentTarget;
		try {
			const { name, email, password } = form;
			const role = form.role as Role;
			if (editing) await request('PUT', `/users/${editing.id}`, { name, email, role });
			else await request('POST', '/users', { name, email, password, role });
			close();
			element.reset();
			toast(t(editing ? 'toast.updated' : 'toast.created', { name: email }));
			// The name shown in the menu is the signed-in user's.
			if (editing?.id === session.user?.id) await invalidateAll();
			await load();
		} catch (e) {
			formError = errorText(e);
		}
	}

	async function remove() {
		try {
			await request('DELETE', `/users/${removing!.id}`);
			toast(t('toast.deleted', { name: removing!.email }));
		} catch (e) {
			actionError = errorText(e);
			return;
		}
		actionError = '';
		await load();
	}
</script>

<PageHeader title={t('nav.users')} description={t('users.desc')}>
	{#snippet actions()}<button type="button" class="btn btn-primary" onclick={() => open()}>{t('users.new')}</button>{/snippet}
</PageHeader>

<p class="mb-3 text-sm text-bad empty:hidden" role="alert">{actionError || loadError}</p>

<DataTable label={t('nav.users')} columns={[t('col.name'), 'Email', 'Role', t('col.actions')]} rows={users}>
	{#snippet cell(user, i)}
		{#if i === 0}<strong>{user.name}</strong>
		{:else if i === 1}{user.email}
		{:else if i === 2}<span class="capitalize">{user.role}</span>
		{:else}<span class="flex flex-wrap items-center justify-end gap-1">
			{#if user.id === session.user?.id}<span class="text-xs text-dim">{t('users.you')}</span>{/if}
			<IconButton icon="edit" label={t('action.edit')} onclick={() => open(user)} />
			{#if user.id !== session.user?.id}<IconButton icon="trash" label={t('action.delete')} onclick={() => { removing = user; confirm.open(); }} danger />{/if}
		</span>{/if}
	{/snippet}
</DataTable>

<Modal bind:this={modal} title={editing ? `${t('action.edit')}: ${editing.email}` : t('users.new')}>
	{#snippet children(close)}
		<form onsubmit={(e) => create(e, close)}>
			<FormField label={t('col.name')}>{#snippet children(a)}<input {...a} class="input" maxlength="100" autocomplete="off" required bind:value={form.name} />{/snippet}</FormField>
			<FormField label="Email">{#snippet children(a)}<input {...a} class="input" type="email" autocomplete="off" required bind:value={form.email} />{/snippet}</FormField>
			{#if !editing}<NewPassword label="Password" bind:value={form.password} />{/if}
			<FormField label="Role">{#snippet children(a)}<Select {...a} disabled={editing?.id === session.user?.id} bind:value={form.role} options={(['viewer', 'operator', 'admin'] as const).map((r) => ({ value: r, label: r, description: t(`role.${r}`) }))} />{/snippet}</FormField>
			<p class="text-sm text-bad empty:hidden" role="alert">{formError}</p>
			<div class="mt-4 flex justify-end gap-2">
				<button type="button" class="btn" onclick={close}>{t('action.cancel')}</button>
				<button type="submit" class="btn btn-primary">{t('action.save')}</button>
			</div>
		</form>
	{/snippet}
</Modal>

<ConfirmDialog bind:this={confirm} title={t('action.delete')} message={t('confirm.delete', { name: removing?.email ?? '' })} confirmLabel={t('action.delete')} onconfirm={remove} />
