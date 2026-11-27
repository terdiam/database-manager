<script lang="ts">
	import { channels } from '#lib/api/index.ts';
	import ResourcePage from '#lib/components/ResourcePage.svelte';
	import Select from '#lib/components/Select.svelte';
	import { t } from '#lib/i18n/index.svelte.ts';

	let type = $state('Telegram');
</script>

<ResourcePage
	brandColumn={1}
	title={t('nav.notifications')}
	description={t('notifications.desc')}
	addLabel={t('notifications.new')}
	columns={[t('col.name'), t('col.type'), t('col.target'), t('col.events')]}
	rows={channels.map((c) => ({ id: c.id, cells: [c.name, c.type, c.target, String(c.events)] }))}
	testLabel={t('notifications.test')}
	fields={[
		{ label: t('col.name') },
		...(type === 'Telegram'
			? [{ label: 'Bot token', type: 'password' as const, help: t('help.secret') }, { label: 'Chat ID' }]
			: [{ label: 'Incoming webhook URL', type: 'url' as const, help: t('help.secret') }])
	]}
>
	{#snippet before()}
		<label class="mb-3 block text-sm font-medium">{t('col.type')}
			<Select class="mt-1" bind:value={type} options={['Telegram', 'Mattermost'].map((v) => ({ value: v, label: v, brand: v }))} />
		</label>
		<fieldset class="mb-3 text-sm">
			<legend class="mb-1 font-medium">{t('col.events')}</legend>
			{#each ['event.backup', 'event.restore', 'event.agent', 'event.chain'] as const as event}
				<label class="flex gap-2"><input type="checkbox" checked />{t(event)}</label>
			{/each}
		</fieldset>
	{/snippet}
</ResourcePage>
