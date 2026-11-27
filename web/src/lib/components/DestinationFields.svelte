<script lang="ts">
	import { t } from '#lib/i18n/index.svelte.ts';
	import type { StorageForm } from '#lib/storage.ts';
	import FormField from './FormField.svelte';
	import LoginFields from './LoginFields.svelte';
	import PasswordInput from './PasswordInput.svelte';

	// The connection of a destination, per kind.
	let { form = $bindable() }: { form: StorageForm } = $props();
</script>

{#if form.kind === 's3'}
	<FormField label="Endpoint" help={t('help.endpoint')}>{#snippet children(a)}<input {...a} class="input" type="url" maxlength="255" placeholder="http://minio:9000" bind:value={form.endpoint} />{/snippet}</FormField>
	<FormField label={t('field.publicEndpoint')} help={t('help.publicEndpoint')}>{#snippet children(a)}<input {...a} class="input" type="url" maxlength="255" bind:value={form.public_endpoint} />{/snippet}</FormField>
	<div class="grid gap-x-3 sm:grid-cols-2">
		<FormField label="Bucket">{#snippet children(a)}<input {...a} class="input" maxlength="255" required bind:value={form.bucket} />{/snippet}</FormField>
		<FormField label="Region">{#snippet children(a)}<input {...a} class="input" maxlength="64" placeholder="us-east-1" bind:value={form.region} />{/snippet}</FormField>
	</div>
{:else if form.kind === 'sftp'}
	<LoginFields bind:form />
{:else}
	<div class="grid gap-x-3 sm:grid-cols-[1fr_8rem]">
		<FormField label="Host" help={t('help.host')}>{#snippet children(a)}<input {...a} class="input" maxlength="255" autocapitalize="none" spellcheck="false" required bind:value={form.host} />{/snippet}</FormField>
		<FormField label="Port">{#snippet children(a)}<input {...a} class="input" type="number" min="1" max="65535" required bind:value={form.port} />{/snippet}</FormField>
	</div>
	{#if form.kind === 'smb'}
		<div class="grid gap-x-3 sm:grid-cols-2">
			<FormField label="Share" help={t('help.share')}>{#snippet children(a)}<input {...a} class="input" maxlength="255" required bind:value={form.bucket} />{/snippet}</FormField>
			<FormField label={t('field.domain')}>{#snippet children(a)}<input {...a} class="input" maxlength="64" autocomplete="off" bind:value={form.domain} />{/snippet}</FormField>
		</div>
	{/if}
	<div class="grid gap-x-3 sm:grid-cols-2">
		<FormField label="Username">{#snippet children(a)}<input {...a} class="input" maxlength="255" autocomplete="off" autocapitalize="none" required bind:value={form.username} />{/snippet}</FormField>
		<FormField label="Password" help={t('help.sealed')}>{#snippet children(a)}<PasswordInput {...a} autocomplete="new-password" required bind:value={form.password} />{/snippet}</FormField>
	</div>
	{#if form.kind === 'ftp'}
		<label class="mb-1 flex items-center gap-2 text-sm"><input type="checkbox" bind:checked={form.tls} />{t('field.ftpTls')}</label>
		<p class="mb-3 text-xs {form.tls ? 'text-dim' : 'text-warn'}">{t(form.tls ? 'help.ftpTls' : 'help.ftpPlain')}</p>
	{/if}
{/if}
<FormField label={t('field.prefix')} help={form.kind === 'sftp' ? t('help.sftpFolder') : ''}>{#snippet children(a)}<input {...a} class="input" maxlength="255" placeholder="backups" bind:value={form.prefix} />{/snippet}</FormField>
{#if form.kind === 's3'}
	<FormField label="Access key">{#snippet children(a)}<input {...a} class="input" autocomplete="off" required bind:value={form.access_key} />{/snippet}</FormField>
	<FormField label="Secret key" help={t('help.sealed')}>{#snippet children(a)}<PasswordInput {...a} autocomplete="new-password" required bind:value={form.secret_key} />{/snippet}</FormField>
	<label class="mb-3 flex items-center gap-2 text-sm"><input type="checkbox" bind:checked={form.virtual_host} />{t('field.virtualHost')}</label>
{/if}
