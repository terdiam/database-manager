<script lang="ts">
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { engines } from '#lib/api/index.ts';
	import { request, type Agent } from '#lib/api/http.ts';
	import BrandIcon from '#lib/components/BrandIcon.svelte';
	import FormField from '#lib/components/FormField.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import PasswordInput from '#lib/components/PasswordInput.svelte';
	import Select from '#lib/components/Select.svelte';
	import Wizard from '#lib/components/Wizard.svelte';
	import { toast } from '#lib/components/toast.svelte.ts';
	import { errorText } from '#lib/errors.ts';
	import { t } from '#lib/i18n/index.svelte.ts';
	import { live } from '#lib/events.ts';
	import { sealFor } from '#lib/secrets.ts';

	let step = $state(0);
	let agents = $state<Agent[]>([]);
	let error = $state('');
	let loadError = $state('');
	let form = $state({ name: '', agent: '', host: '', port: 5432, username: 'postgres', password: '', ssl_mode: 'prefer' });
	const agent = $derived(agents.find((a) => a.id === form.agent));

	async function load() {
		try {
			agents = (await request<Agent[]>('GET', '/agents')).filter((a) => a.key);
			if (!agents.some((a) => a.id === form.agent)) form.agent = agents[0]?.id ?? '';
			loadError = '';
		} catch (e) {
			loadError = errorText(e);
		}
	}
	onMount(() => {
		load();
		return live(['agents'], load);
	});

	async function save() {
		const id = crypto.randomUUID();
		const { name, host, port, username, password, ssl_mode } = form;
		try {
			const secret = await sealFor(agent, 'database', id, { host: host.trim(), port, username, password, ssl_mode });
			await request('POST', '/database-servers', { id, name, agent_id: form.agent, host, port, username, ssl_mode, secret });
			toast(t('toast.created', { name }));
			await goto(resolve('databases'));
		} catch (e) {
			error = errorText(e);
		}
	}
</script>

<PageHeader title={t('databases.new')} />
<p class="mb-3 text-sm text-bad empty:hidden" role="alert">{error || loadError}</p>

<Wizard bind:step steps={[t('wizard.engine'), t('wizard.connection'), t('wizard.summary')]} finishLabel={t('action.save')} canFinish={!!agent} onfinish={save}>
	{#if step === 0}
		<fieldset>
			<legend class="mb-3 text-sm font-semibold">{t('wizard.engine')}</legend>
			<div class="grid grid-cols-2 gap-2 sm:grid-cols-4">
				{#each Object.entries(engines) as [key, info]}
					<label class="rounded-lg border border-line p-3 text-center text-sm font-medium has-checked:border-brand has-checked:bg-muted has-disabled:opacity-50">
						<input type="radio" name="engine" class="mb-1 block w-full" checked={key === 'postgresql'} disabled={key !== 'postgresql'} />
						<span class="mx-auto my-1 block w-fit"><BrandIcon name={key} size={28} /></span>
						{info.label}
						{#if key !== 'postgresql'}<span class="block text-xs font-normal text-dim">{t('soon')}</span>{/if}
					</label>
				{/each}
			</div>
		</fieldset>
	{:else if step === 1}
		{#if !agents.length}<p class="mb-3 text-sm text-warn" role="alert">{t('databases.noAgent')} <a class="underline" href={resolve('agents')}>{t('nav.agents')}</a></p>{/if}
		<div class="grid gap-x-3 sm:grid-cols-2">
			<FormField label={t('col.name')} help={t('help.name')}>{#snippet children(a)}<input {...a} class="input" maxlength="64" required bind:value={form.name} />{/snippet}</FormField>
			<FormField label={t('field.agent')} help={t('help.agent')}>{#snippet children(a)}<Select {...a} required bind:value={form.agent} options={agents.map((x) => ({ value: x.id, label: x.name, icon: 'server' as const, description: t(`status.${x.online ? 'online' : 'offline'}`) }))} />{/snippet}</FormField>
		</div>
		<div class="grid gap-x-3 sm:grid-cols-[1fr_8rem]">
			<FormField label="Host" help={t('help.host')}>{#snippet children(a)}<input {...a} class="input" maxlength="255" required bind:value={form.host} />{/snippet}</FormField>
			<FormField label="Port">{#snippet children(a)}<input {...a} class="input" type="number" min="1" max="65535" required bind:value={form.port} />{/snippet}</FormField>
		</div>
		<div class="grid gap-x-3 sm:grid-cols-2">
			<FormField label="Username">{#snippet children(a)}<input {...a} class="input" maxlength="255" autocomplete="off" required bind:value={form.username} />{/snippet}</FormField>
			<FormField label="Password" help={t('help.sealed')}>{#snippet children(a)}<PasswordInput {...a} autocomplete="new-password" bind:value={form.password} />{/snippet}</FormField>
		</div>
		<FormField label="SSL mode">{#snippet children(a)}<Select {...a} bind:value={form.ssl_mode} options={['disable', 'prefer', 'require', 'verify-full'].map((v) => ({ value: v, label: v }))} />{/snippet}</FormField>
	{:else}
		<dl class="grid grid-cols-[10rem_1fr] gap-2 text-sm">
			<dt class="text-dim">{t('col.name')}</dt><dd>{form.name}</dd>
			<dt class="text-dim">{t('col.address')}</dt><dd>{form.username}@{form.host}:{form.port}</dd>
			<dt class="text-dim">SSL mode</dt><dd>{form.ssl_mode}</dd>
			<dt class="text-dim">{t('field.agent')}</dt><dd>{agent?.name} <code class="text-xs text-dim">{agent?.fingerprint}</code></dd>
		</dl>
		<p class="mt-3 text-sm text-dim">{t('help.fingerprint')}</p>
	{/if}
</Wizard>
