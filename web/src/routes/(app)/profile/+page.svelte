<script lang="ts">
	import SplitTitle from '#lib/components/SplitTitle.svelte';
	import { request } from '#lib/api/http.ts';
	import Avatar from '#lib/components/Avatar.svelte';
	import NewPassword from '#lib/components/NewPassword.svelte';
	import PasswordInput from '#lib/components/PasswordInput.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import { toast } from '#lib/components/toast.svelte.ts';
	import { errorText } from '#lib/errors.ts';
	import { t } from '#lib/i18n/index.svelte.ts';
	import { session } from '#lib/session.svelte.ts';

	const currentUser = $derived(session.user ?? { name: '', email: '', role: '' });
	let current = $state('');
	let next = $state('');
	let error = $state('');

	async function save(event: SubmitEvent & { currentTarget: HTMLFormElement }) {
		event.preventDefault();
		const form = event.currentTarget;
		try {
			await request('POST', '/auth/password', { current, new: next });
			// reset() also clears the browser's red "already touched" state, not just the values.
			form.reset();
			current = next = error = '';
			toast(t('toast.password'));
		} catch (e) {
			error = errorText(e);
		}
	}
</script>

<PageHeader title={t('menu.profile')} />

<section class="card mb-3 flex items-center gap-4">
	<Avatar name={currentUser.name} size={56} />
	<div class="min-w-0">
		<p class="truncate font-semibold">{currentUser.name}</p>
		<p class="truncate text-sm text-dim">{currentUser.email}</p>
		<p class="text-xs text-dim capitalize">{currentUser.role}</p>
	</div>
</section>

<form class="card" onsubmit={save}>
	<SplitTitle text={t('profile.password')} class="mb-3 font-semibold" />
	<label for="current-password" class="mb-1 block text-sm font-medium">{t('profile.current')}</label>
	<PasswordInput id="current-password" class="mb-3" autocomplete="current-password" required bind:value={current} />
	<NewPassword label={t('profile.new')} bind:value={next} />
	<p class="mb-3 text-sm text-bad empty:hidden" role="alert">{error}</p>
	<button type="submit" class="btn btn-primary">{t('action.save')}</button>
</form>
