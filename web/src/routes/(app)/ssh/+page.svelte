<script lang="ts">
	import { sshConfigs } from '#lib/api/index.ts';
	import ResourcePage from '#lib/components/ResourcePage.svelte';
	import { t } from '#lib/i18n/index.svelte.ts';
</script>

<ResourcePage
	title={t('nav.ssh')}
	description={t('ssh.desc')}
	addLabel={t('ssh.new')}
	columns={[t('col.name'), t('col.address'), 'User', t('col.auth'), t('col.usedBy')]}
	rows={sshConfigs.map((s) => ({ id: s.id, cells: [s.name, `${s.host}:${s.port}`, s.user, s.auth, `${s.used} database`] }))}
	fields={[
		{ label: t('col.name') },
		{ label: 'Host' },
		{ label: 'Port', type: 'number', value: '22' },
		{ label: 'User' },
		{ label: t('col.auth'), options: ['Private key', 'Password'] },
		{ label: t('ssh.secret'), type: 'textarea', help: t('help.secret') }
	]}
/>
