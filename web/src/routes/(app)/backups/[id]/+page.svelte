<script lang="ts">
	import { page } from '$app/state';
	import { request, type Backup } from '#lib/api/http.ts';
	import BackupForm from '#lib/components/BackupForm.svelte';
	import EmptyState from '#lib/components/EmptyState.svelte';
	import { errorText } from '#lib/errors.ts';
	import { t } from '#lib/i18n/index.svelte.ts';

	const found = request<Backup[]>('GET', '/backups').then((list) => list.find((b) => b.id === page.params.id));
</script>

{#await found}
	<p class="text-sm text-dim">{t('login.checking')}</p>
{:then backup}
	{#if backup}<BackupForm {backup} />{:else}<EmptyState title={t('error.not_found')} />{/if}
{:catch error}
	<p class="text-sm text-bad" role="alert">{errorText(error)}</p>
{/await}
