<script lang="ts">
	import { engines, loginOptional, tlsModes, type Engine } from '#lib/api/index.ts';
	import { t } from '#lib/i18n/index.svelte.ts';
	import FormField from './FormField.svelte';
	import PasswordInput from './PasswordInput.svelte';
	import Select from './Select.svelte';

	// How to reach a database, per engine. Used to add one and to change its connection.
	let { engine, form = $bindable() }: { engine: Engine; form: { host: string; port: number; username: string; password: string; ssl_mode: string } } = $props();

	const modes = $derived(tlsModes(engine));
	const userHelp = $derived(engine === 'mongodb' ? t('help.mongoUser') : loginOptional(engine) ? t('help.optionalLogin') : '');
</script>

{#if engine === 'sqlite'}
	<FormField label={t('field.folder')} help={t('help.folder')}>{#snippet children(a)}<input {...a} class="input" maxlength="255" autocomplete="off" spellcheck="false" placeholder="app" bind:value={form.host} />{/snippet}</FormField>
{:else}
	<div class="grid gap-x-3 sm:grid-cols-[1fr_8rem]">
		<FormField label="Host" help={t('help.host')}>{#snippet children(a)}<input {...a} class="input" maxlength="255" required bind:value={form.host} />{/snippet}</FormField>
		<FormField label="Port">{#snippet children(a)}<input {...a} class="input" type="number" min="1" max="65535" required placeholder={String(engines[engine].port)} bind:value={form.port} />{/snippet}</FormField>
	</div>
	<div class="grid gap-x-3 sm:grid-cols-2">
		<FormField label="Username" help={userHelp}>{#snippet children(a)}<input {...a} class="input" maxlength="255" autocomplete="off" required={!loginOptional(engine)} bind:value={form.username} />{/snippet}</FormField>
		<FormField label="Password" help={t('help.sealed')}>{#snippet children(a)}<PasswordInput {...a} autocomplete="new-password" bind:value={form.password} />{/snippet}</FormField>
	</div>
	<FormField label="SSL mode">{#snippet children(a)}<Select {...a} required placeholder={t('field.choose')} bind:value={form.ssl_mode} options={modes.map((v) => ({ value: v, label: v }))} />{/snippet}</FormField>
{/if}
