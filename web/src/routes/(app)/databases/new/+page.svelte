<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { agents, engines, sshConfigs, type Engine } from '#lib/api/index.ts';
	import BrandIcon from '#lib/components/BrandIcon.svelte';
	import FormField from '#lib/components/FormField.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import Select from '#lib/components/Select.svelte';
	import Wizard from '#lib/components/Wizard.svelte';
	import { toast } from '#lib/components/toast.svelte.ts';
	import { t } from '#lib/i18n/index.svelte.ts';

	let step = $state(0);
	let engine = $state<Engine>('postgresql');
	let form = $state({ name: '', host: '', port: 5432, user: '', password: '', path: '', agent: 'local', ssh: '', selection: 'all', databases: '' });
	let tested = $state(false);
	const isFile = $derived(engine === 'sqlite');

	function pick(next: Engine) {
		engine = next;
		form.port = engines[next].port;
		tested = false;
	}
</script>

<PageHeader title={t('databases.new')} />

<Wizard
	bind:step
	steps={[t('wizard.engine'), t('wizard.connection'), t('wizard.select'), t('wizard.summary')]}
	finishLabel={t('action.save')}
	onfinish={() => { toast(t('toast.saved')); goto(resolve('databases')); }}
>
	{#if step === 0}
		<fieldset>
			<legend class="mb-3 text-sm font-semibold">{t('wizard.engine')}</legend>
			<div class="grid grid-cols-2 gap-2 sm:grid-cols-4">
				{#each Object.entries(engines) as [key, info]}
					<label class="rounded-lg border border-line p-3 text-center text-sm font-medium has-checked:border-brand has-checked:bg-muted">
						<input type="radio" name="engine" class="mb-1 block w-full" checked={engine === key} onchange={() => pick(key as Engine)} />
						<span class="mx-auto my-1 block w-fit"><BrandIcon name={key} size={28} /></span>
						{info.label}
						<span class="block text-xs font-normal text-dim">{info.incremental ? 'full + incremental' : 'full'}</span>
					</label>
				{/each}
			</div>
		</fieldset>
	{:else if step === 1}
		<FormField label={t('col.name')} help={t('help.name')}>{#snippet children(a)}<input {...a} class="input" required bind:value={form.name} />{/snippet}</FormField>
		<div class="grid gap-x-3 sm:grid-cols-2">
			<FormField label={t('field.agent')} help={t('help.agent')}>{#snippet children(a)}<Select {...a} bind:value={form.agent} options={agents.map((x) => ({ value: x.name, label: x.name, icon: 'server' as const, description: t(`status.${x.status}`) }))} />{/snippet}</FormField>
			<FormField label={t('field.ssh')} help={t('help.ssh')}>{#snippet children(a)}<Select {...a} bind:value={form.ssh} options={[{ value: '', label: t('field.noSsh') }, ...sshConfigs.map((x) => ({ value: x.name, label: x.name, icon: 'key' as const, description: `${x.user}@${x.host}:${x.port}` }))]} />{/snippet}</FormField>
		</div>
		{#if isFile}
			<FormField label={t('field.path')} help={t('help.path')}>{#snippet children(a)}<input {...a} class="input" required placeholder="/var/lib/app/data.db" bind:value={form.path} />{/snippet}</FormField>
		{:else}
			<div class="grid gap-x-3 sm:grid-cols-[1fr_8rem]">
				<FormField label="Host">{#snippet children(a)}<input {...a} class="input" required bind:value={form.host} />{/snippet}</FormField>
				<FormField label="Port">{#snippet children(a)}<input {...a} class="input" type="number" min="1" max="65535" required bind:value={form.port} />{/snippet}</FormField>
			</div>
			<div class="grid gap-x-3 sm:grid-cols-2">
				<FormField label="Username">{#snippet children(a)}<input {...a} class="input" autocomplete="off" bind:value={form.user} />{/snippet}</FormField>
				<FormField label="Password" help={t('help.secret')}>{#snippet children(a)}<input {...a} class="input" type="password" autocomplete="new-password" bind:value={form.password} />{/snippet}</FormField>
			</div>
		{/if}
		<button type="button" class="btn" onclick={() => (tested = true)}>{t('action.test')}</button>
		<p class="mt-2 text-sm text-ok" role="status">{tested ? t('databases.testOk', { engine: engines[engine].label }) : ''}</p>
	{:else if step === 2}
		<fieldset class="text-sm">
			<legend class="mb-2 font-semibold">{t('wizard.select')}</legend>
			<label class="mb-1 flex gap-2"><input type="radio" value="all" bind:group={form.selection} />{t('select.all')}</label>
			<label class="flex gap-2"><input type="radio" value="some" bind:group={form.selection} />{t('select.some')}</label>
		</fieldset>
		{#if form.selection === 'some'}
			<div class="mt-3"><FormField label={t('select.names')}>{#snippet children(a)}<input {...a} class="input" required placeholder="shop, billing" bind:value={form.databases} />{/snippet}</FormField></div>
		{/if}
	{:else}
		<dl class="grid grid-cols-[10rem_1fr] gap-2 text-sm">
			<dt class="text-dim">{t('col.name')}</dt><dd>{form.name}</dd>
			<dt class="text-dim">{t('col.engine')}</dt><dd>{engines[engine].label}</dd>
			<dt class="text-dim">{t('col.address')}</dt><dd>{isFile ? form.path : `${form.host}:${form.port}`}</dd>
			<dt class="text-dim">{t('col.via')}</dt><dd>{form.agent}{form.ssh ? ` → ${form.ssh}` : ''}</dd>
			<dt class="text-dim">{t('wizard.select')}</dt><dd>{form.selection === 'all' ? t('select.all') : form.databases}</dd>
		</dl>
	{/if}
</Wizard>
