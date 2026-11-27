<script lang="ts">
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { connectionOf, engines, supported, type Engine } from '#lib/api/index.ts';
	import { request, type Agent, type SshConfig } from '#lib/api/http.ts';
	import BrandIcon from '#lib/components/BrandIcon.svelte';
	import ConnectionFields from '#lib/components/ConnectionFields.svelte';
	import FormField from '#lib/components/FormField.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import Select from '#lib/components/Select.svelte';
	import Wizard from '#lib/components/Wizard.svelte';
	import { toast } from '#lib/components/toast.svelte.ts';
	import { errorText } from '#lib/errors.ts';
	import { t } from '#lib/i18n/index.svelte.ts';
	import { live } from '#lib/events.ts';
	import { sealFor } from '#lib/secrets.ts';

	let step = $state(0);
	let engine = $state<Engine>('postgresql');
	let agents = $state<Agent[]>([]);
	let tunnels = $state<SshConfig[]>([]);
	let error = $state('');
	let loadError = $state('');
	let form = $state({ name: '', agent: '', host: '', port: 5432, username: 'postgres', password: '', ssl_mode: 'prefer', ssh_config_id: '' });
	const agent = $derived(agents.find((a) => a.id === form.agent));

	async function load() {
		try {
			const [all, configs] = await Promise.all([request<Agent[]>('GET', '/agents'), request<SshConfig[]>('GET', '/ssh-configs')]);
			agents = all.filter((a) => a.key);
			tunnels = configs;
			if (!agents.some((a) => a.id === form.agent)) form.agent = agents[0]?.id ?? '';
			loadError = '';
		} catch (e) {
			loadError = errorText(e);
		}
	}
	onMount(() => {
		load();
		return live(['agents', 'ssh_configs'], load);
	});

	// Each engine has its own usual port, account and TLS default.
	function pick(next: Engine) {
		engine = next;
		const username = { postgresql: 'postgres', mysql: 'root', mariadb: 'root', mongodb: 'root' }[next as 'mysql'] ?? '';
		Object.assign(form, { host: '', port: engines[next].port, username, password: '', ssl_mode: next === 'mongodb' ? 'disable' : 'prefer', ssh_config_id: '' });
	}

	async function save() {
		const id = crypto.randomUUID();
		const { name } = form;
		const connection = connectionOf(engine, form);
		try {
			const secret = await sealFor(agent, 'database', id, { engine, ...connection });
			const { host, port, username, ssl_mode } = connection;
			// A tunnel of another agent is dropped: the choice follows the agent.
			const ssh_config_id = tunnels.find((c) => c.id === form.ssh_config_id && c.agent_id === form.agent && engine !== 'sqlite')?.id;
			await request('POST', '/database-servers', { id, name, engine, agent_id: form.agent, host, port, username, ssl_mode, secret, ssh_config_id });
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
						<input type="radio" name="engine" class="mb-1 block w-full" checked={engine === key} disabled={!supported.includes(key as Engine)} onchange={() => pick(key as Engine)} />
						<span class="mx-auto my-1 block w-fit"><BrandIcon name={key} size={28} /></span>
						{info.label}
						{#if !supported.includes(key as Engine)}<span class="block text-xs font-normal text-dim">{t('soon')}</span>{/if}
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
		<ConnectionFields {engine} bind:form />
		{#if engine !== 'sqlite'}<FormField label="SSH tunnel" help={t('help.tunnel')}>{#snippet children(a)}<Select {...a} bind:value={form.ssh_config_id} options={[{ value: '', label: t('field.noTunnel') }, ...tunnels.filter((c) => c.agent_id === form.agent).map((c) => ({ value: c.id, label: c.name, icon: 'key' as const, description: `${c.username}@${c.host}` }))]} />{/snippet}</FormField>{/if}
	{:else}
		<dl class="grid grid-cols-[10rem_1fr] gap-2 text-sm">
			<dt class="text-dim">{t('col.name')}</dt><dd>{form.name}</dd>
			<dt class="text-dim">{t('col.engine')}</dt><dd>{engines[engine].label}</dd>
			{#if engine === 'sqlite'}
				<dt class="text-dim">{t('field.folder')}</dt><dd>/{form.host}</dd>
			{:else}
				<dt class="text-dim">{t('col.address')}</dt><dd>{form.username ? `${form.username}@` : ''}{form.host}:{form.port}</dd>
				<dt class="text-dim">SSL mode</dt><dd>{form.ssl_mode}</dd>
			{/if}
			{#if engine !== 'sqlite'}<dt class="text-dim">SSH tunnel</dt><dd>{tunnels.find((c) => c.id === form.ssh_config_id && c.agent_id === form.agent)?.name ?? t('field.noTunnel')}</dd>{/if}
			<dt class="text-dim">{t('field.agent')}</dt><dd>{agent?.name} <code class="text-xs text-dim">{agent?.fingerprint}</code></dd>
		</dl>
		<p class="mt-3 text-sm text-dim">{t('help.fingerprint')}</p>
	{/if}
</Wizard>
