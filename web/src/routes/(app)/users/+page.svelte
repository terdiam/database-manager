<script lang="ts">
	import { onMount } from 'svelte';
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

	function open() {
		form = blank();
		formError = '';
		modal.open();
	}

	async function create(event: SubmitEvent & { currentTarget: HTMLFormElement }, close: () => void) {
		event.preventDefault();
		const element = event.currentTarget;
		try {
			await request('POST', '/users', { ...form, role: form.role as Role });
			close();
			element.reset();
			toast(t('toast.created', { name: form.email }));
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
	{#snippet actions()}<button type="button" class="btn btn-primary" onclick={open}>{t('users.new')}</button>{/snippet}
</PageHeader>

<p class="mb-3 text-sm text-bad empty:hidden" role="alert">{actionError || loadError}</p>

<DataTable label={t('nav.users')} columns={[t('col.name'), 'Email', 'Role', t('col.actions')]} rows={users}>
	{#snippet cell(user, i)}
		{#if i === 0}<strong>{user.name}</strong>
		{:else if i === 1}{user.email}
		{:else if i === 2}<span class="capitalize">{user.role}</span>
		{:else if user.id === session.user?.id}<span class="text-xs text-dim">{t('users.you')}</span>
		{:else}<button type="button" class="btn btn-danger" onclick={() => { removing = user; confirm.open(); }}>{t('action.delete')}</button>{/if}
	{/snippet}
</DataTable>

<Modal bind:this={modal} title={t('users.new')}>
	{#snippet children(close)}
		<form onsubmit={(e) => create(e, close)}>
			<FormField label={t('col.name')}>{#snippet children(a)}<input {...a} class="input" maxlength="100" autocomplete="off" required bind:value={form.name} />{/snippet}</FormField>
			<FormField label="Email">{#snippet children(a)}<input {...a} class="input" type="email" autocomplete="off" required bind:value={form.email} />{/snippet}</FormField>
			<NewPassword label="Password" bind:value={form.password} />
			<FormField label="Role">{#snippet children(a)}<Select {...a} bind:value={form.role} options={(['viewer', 'operator', 'admin'] as const).map((r) => ({ value: r, label: r, description: t(`role.${r}`) }))} />{/snippet}</FormField>
			<p class="text-sm text-bad empty:hidden" role="alert">{formError}</p>
			<div class="mt-4 flex justify-end gap-2">
				<button type="button" class="btn" onclick={close}>{t('action.cancel')}</button>
				<button type="submit" class="btn btn-primary">{t('action.save')}</button>
			</div>
		</form>
	{/snippet}
</Modal>

<ConfirmDialog bind:this={confirm} title={t('action.delete')} message={t('confirm.delete', { name: removing?.email ?? '' })} confirmLabel={t('action.delete')} onconfirm={remove} />
