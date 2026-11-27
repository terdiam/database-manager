<script lang="ts">
	import { destinations } from '#lib/api/index.ts';
	import ResourcePage from '#lib/components/ResourcePage.svelte';
	import Select from '#lib/components/Select.svelte';
	import { bytes } from '#lib/format.ts';
	import { t } from '#lib/i18n/index.svelte.ts';

	let type = $state('S3 / MinIO');
	const text = (label: string) => ({ label });
	const secret = (label: string) => ({ label, type: 'password' as const, help: t('help.secret') });
	const extra = $derived(
		{
			'S3 / MinIO': [text('Endpoint'), text('Bucket'), text('Region'), text('Access key'), secret('Secret key')],
			Samba: [text('Host'), text('Share'), text('Folder'), text('User'), secret('Password')],
			'SFTP / SSH': [text('Host'), text('Port'), text('Folder'), text('User'), secret('Password / key')],
			'FTP / FTPS': [text('Host'), text('Port'), text('Folder'), text('User'), secret('Password')],
			'Google Drive': [text('Client ID'), secret('Client secret'), secret('Refresh token'), text('Folder ID')]
		}[type]!
	);
</script>

<ResourcePage
	brandColumn={1}
	title={t('nav.destinations')}
	description={t('destinations.desc')}
	addLabel={t('destinations.new')}
	columns={[t('col.name'), t('col.type'), t('col.target'), t('col.size'), t('col.status')]}
	rows={destinations.map((d) => ({ id: d.id, cells: [d.name, d.type, d.target, bytes(d.used)], status: d.status }))}
	fields={[{ label: t('col.name') }, ...extra]}
>
	{#snippet before()}
		<label class="mb-3 block text-sm font-medium">{t('col.type')}
			<Select class="mt-1" bind:value={type} options={['S3 / MinIO', 'Samba', 'SFTP / SSH', 'FTP / FTPS', 'Google Drive'].map((v) => ({ value: v, label: v, brand: v }))} />
		</label>
		{#if type === 'Google Drive'}<p class="mb-3 text-sm text-warn">{t('destinations.gdrive')}</p>{/if}
	{/snippet}
</ResourcePage>
