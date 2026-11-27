<script lang="ts">
	import { t } from '#lib/i18n/index.svelte.ts';
	import FormField from './FormField.svelte';
	import PasswordInput from './PasswordInput.svelte';

	// Signing in over SSH, for tunnels and SFTP: host, account, password or key, host key.
	let {
		form = $bindable()
	}: { form: { host: string; port: number; username: string; auth: string; password: string; private_key: string; passphrase: string; fingerprint: string } } = $props();
	const id = $props.id();
</script>

<div class="grid gap-x-3 sm:grid-cols-[1fr_8rem]">
	<FormField label="Host" help={t('help.host')}>{#snippet children(a)}<input {...a} class="input" maxlength="255" autocapitalize="none" spellcheck="false" required bind:value={form.host} />{/snippet}</FormField>
	<FormField label="Port">{#snippet children(a)}<input {...a} class="input" type="number" min="1" max="65535" required bind:value={form.port} />{/snippet}</FormField>
</div>
<FormField label="Username">{#snippet children(a)}<input {...a} class="input" maxlength="255" autocomplete="off" autocapitalize="none" spellcheck="false" required bind:value={form.username} />{/snippet}</FormField>
<fieldset class="mb-3">
	<legend class="mb-1 text-sm font-medium">{t('col.auth')}</legend>
	<div class="flex flex-wrap gap-4 text-sm">
		{#each ['password', 'key'] as const as method}
			<label class="flex items-center gap-1.5"><input type="radio" name="{id}-auth" value={method} bind:group={form.auth} />{t(`auth.${method}`)}</label>
		{/each}
	</div>
</fieldset>
{#if form.auth === 'key'}
	<FormField label="Private key" help={t('help.privateKey')}>{#snippet children(a)}<textarea {...a} class="input py-2 font-mono text-xs" rows="5" maxlength="5000" autocomplete="off" spellcheck="false" required bind:value={form.private_key}></textarea>{/snippet}</FormField>
	<FormField label="Passphrase" help={t('help.passphrase')}>{#snippet children(a)}<PasswordInput {...a} autocomplete="new-password" bind:value={form.passphrase} />{/snippet}</FormField>
{:else}
	<FormField label="Password" help={t('help.sealed')}>{#snippet children(a)}<PasswordInput {...a} autocomplete="new-password" required bind:value={form.password} />{/snippet}</FormField>
{/if}
<FormField label={t('field.fingerprint')} help={t('help.hostKey')}>{#snippet children(a)}<input {...a} class="input font-mono" maxlength="100" placeholder="SHA256:…" pattern={'SHA256:[A-Za-z0-9+\\/=]+'} autocomplete="off" spellcheck="false" bind:value={form.fingerprint} />{/snippet}</FormField>
