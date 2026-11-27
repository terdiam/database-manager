<script lang="ts">
	import { request } from '#lib/api/http.ts';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import PasswordInput from '#lib/components/PasswordInput.svelte';
	import SplitTitle from '#lib/components/SplitTitle.svelte';
	import { errorText } from '#lib/errors.ts';
	import { t } from '#lib/i18n/index.svelte.ts';

	let password = $state('');
	let error = $state('');
	let busy = $state(false);

	// The keys are only ever written to a file on this computer.
	async function download(event: SubmitEvent & { currentTarget: HTMLFormElement }) {
		event.preventDefault();
		const form = event.currentTarget;
		busy = true;
		try {
			const keys = await request<{ recipient: string; identity: string }[]>('POST', '/backup-keys/export', { password });
			const text = keys.map((k) => `# public key: ${k.recipient}\n${k.identity}\n`).join('');
			const link = document.createElement('a');
			link.href = URL.createObjectURL(new Blob([text], { type: 'text/plain' }));
			link.download = 'dbm-backup-keys.txt';
			link.click();
			URL.revokeObjectURL(link.href);
			form.reset();
			password = error = '';
		} catch (e) {
			error = errorText(e);
		} finally {
			busy = false;
		}
	}
</script>

<PageHeader title={t('nav.settings')} />

<form class="card" onsubmit={download}>
	<SplitTitle text={t('settings.keys')} class="mb-1 font-semibold" />
	<p class="mb-3 text-sm text-dim">{t('settings.keysHint')}</p>
	<label for="export-password" class="mb-1 block text-sm font-medium">{t('profile.current')}</label>
	<PasswordInput id="export-password" class="mb-3" autocomplete="current-password" required bind:value={password} />
	<p class="mb-3 text-sm text-bad empty:hidden" role="alert">{error}</p>
	<button type="submit" class="btn btn-primary" disabled={busy}>{t('settings.download')}</button>
</form>
