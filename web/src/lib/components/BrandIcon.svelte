<script lang="ts" module>
	import {
		siElasticsearch, siGoogledrive, siMariadb, siMattermost, siMinio, siMongodb,
		siMysql, siOpensearch, siPostgresql, siRedis, siSqlite, siTelegram
	} from 'simple-icons';

	// Logos: simple-icons (CC0). Marks belong to their owners. Samba/SFTP/FTP have none, so they get a text badge.
	const logos = {
		postgresql: siPostgresql, mysql: siMysql, mariadb: siMariadb, mongodb: siMongodb, sqlite: siSqlite,
		redis: siRedis, opensearch: siOpensearch, elasticsearch: siElasticsearch,
		'S3 / MinIO': siMinio, 'Google Drive': siGoogledrive, Telegram: siTelegram, Mattermost: siMattermost
	};
	const badges: Record<string, string> = { Samba: 'SMB', 'SFTP / SSH': 'SSH', 'FTP / FTPS': 'FTP' };
</script>

<script lang="ts">
	let { name, size = 20 }: { name: string; size?: number } = $props();
	const logo = $derived(logos[name as keyof typeof logos]);
</script>

<!-- Fixed light tile keeps dark brand colors readable in dark mode. -->
<span class="inline-grid shrink-0 place-items-center rounded-md border border-line bg-white" style:width="{size + 8}px" style:height="{size + 8}px" aria-hidden="true">
	{#if logo}
		<svg width={size} height={size} viewBox="0 0 24 24" fill="#{logo.hex}"><path d={logo.path} /></svg>
	{:else}
		<span class="font-bold text-slate-700" style:font-size="{size * 0.45}px">{badges[name] ?? '?'}</span>
	{/if}
</span>
