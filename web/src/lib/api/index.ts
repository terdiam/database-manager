// Sample data for the pages that have no backend yet.
export type Engine = 'postgresql' | 'mysql' | 'mariadb' | 'mongodb' | 'sqlite' | 'redis' | 'opensearch' | 'elasticsearch';
export type Status = 'completed' | 'running' | 'failed' | 'pending' | 'online' | 'offline';

export const engines: Record<Engine, { label: string; port: number; incremental: boolean }> = {
	postgresql: { label: 'PostgreSQL', port: 5432, incremental: true },
	mysql: { label: 'MySQL', port: 3306, incremental: true },
	mariadb: { label: 'MariaDB', port: 3306, incremental: true },
	mongodb: { label: 'MongoDB', port: 27017, incremental: true },
	sqlite: { label: 'SQLite', port: 0, incremental: false },
	redis: { label: 'Redis', port: 6379, incremental: false },
	opensearch: { label: 'OpenSearch', port: 9200, incremental: false },
	elasticsearch: { label: 'Elasticsearch', port: 9200, incremental: false }
};

const now = Date.now();
const ago = (hours: number) => new Date(now - hours * 3_600_000).toISOString();
const GB = 1024 ** 3;

export const agents = [
	{ id: 'a1', name: 'local', status: 'online' as Status, version: '0.1.0', seen: ago(0.002), jobs: 1 },
	{ id: 'a2', name: 'dc-jakarta', status: 'online' as Status, version: '0.1.0', seen: ago(0.004), jobs: 0 },
	{ id: 'a3', name: 'branch-surabaya', status: 'offline' as Status, version: '0.1.0', seen: ago(5), jobs: 0 }
];

export const sshConfigs = [
	{ id: 's1', name: 'bastion-prod', host: 'bastion.example.com', port: 22, user: 'backup', auth: 'key', used: 2 },
	{ id: 's2', name: 'jump-staging', host: '10.8.0.4', port: 2222, user: 'deploy', auth: 'password', used: 1 }
];

export const servers = [
	{ id: 'd1', name: 'prod-pg', engine: 'postgresql' as Engine, address: '10.0.0.12:5432', ssh: 'bastion-prod', agent: 'local', status: 'online' as Status },
	{ id: 'd2', name: 'prod-mysql', engine: 'mysql' as Engine, address: '10.0.0.20:3306', ssh: 'bastion-prod', agent: 'local', status: 'offline' as Status },
	{ id: 'd3', name: 'crm-mongo', engine: 'mongodb' as Engine, address: 'mongo.internal:27017', ssh: '', agent: 'dc-jakarta', status: 'online' as Status },
	{ id: 'd4', name: 'cache-redis', engine: 'redis' as Engine, address: '10.0.0.31:6379', ssh: 'jump-staging', agent: 'local', status: 'online' as Status },
	{ id: 'd5', name: 'search-logs', engine: 'opensearch' as Engine, address: 'os.internal:9200', ssh: '', agent: 'dc-jakarta', status: 'online' as Status }
];

export const destinations = [
	{ id: 't1', name: 'minio-utama', type: 'S3 / MinIO', target: 's3://backups/prod', used: 412 * GB, status: 'online' as Status },
	{ id: 't2', name: 'nas-kantor', type: 'Samba', target: '//nas/backup', used: 96 * GB, status: 'online' as Status },
	{ id: 't3', name: 'offsite-sftp', type: 'SFTP / SSH', target: 'sftp://offsite.example.com/db', used: 188 * GB, status: 'online' as Status },
	{ id: 't4', name: 'arsip-drive', type: 'Google Drive', target: 'My Drive/db-archive', used: 12 * GB, status: 'offline' as Status }
];

export type BackupType = 'full' | 'incremental';

/** One entry = one type with one schedule. An incremental entry points at its full backup through `base`. */
export type Backup = {
	id: string; name: string; type: BackupType; base?: string; method?: 'logical' | 'physical';
	server: string; databases: string; destinations: string[];
	schedule: 'hours' | 'daily' | 'weekly' | 'monthly' | 'yearly' | 'cron';
	retention: { amount: number; unit: 'days' | 'months' }; compression: string; next: string;
};

const pg = { server: 'prod-pg', databases: 'all', destinations: ['minio-utama', 'offsite-sftp'], retention: { amount: 3, unit: 'months' as const }, compression: 'zstd' };
const shop = { server: 'prod-mysql', databases: 'shop, billing', destinations: ['minio-utama'], retention: { amount: 30, unit: 'days' as const }, compression: 'zstd' };

export const backups: Backup[] = [
	{ id: 'b1', name: 'prod-pg full bulanan', type: 'full', method: 'physical', schedule: 'monthly', next: ago(-90), ...pg },
	{ id: 'b2', name: 'prod-pg incremental', type: 'incremental', base: 'b1', schedule: 'hours', next: ago(-2), ...pg },
	{ id: 'b3', name: 'toko full mingguan', type: 'full', schedule: 'weekly', next: ago(-60), ...shop },
	{ id: 'b4', name: 'toko incremental', type: 'incremental', base: 'b3', schedule: 'hours', next: ago(-5), ...shop },
	{ id: 'b5', name: 'crm full mingguan', type: 'full', schedule: 'weekly', server: 'crm-mongo', databases: 'all', destinations: ['nas-kantor'], retention: { amount: 6, unit: 'months' }, compression: 'gzip', next: ago(-60) },
	{ id: 'b6', name: 'redis harian', type: 'full', schedule: 'daily', server: 'cache-redis', databases: 'all', destinations: ['nas-kantor'], retention: { amount: 7, unit: 'days' }, compression: 'lz4', next: ago(-11) }
];

/** PostgreSQL needs a physical full backup; MongoDB's oplog needs a whole-instance one. */
export function canBeBase(b: Backup): boolean {
	const engine = servers.find((s) => s.name === b.server)?.engine;
	if (b.type !== 'full' || !engine || !engines[engine].incremental) return false;
	if (engine === 'postgresql') return b.method === 'physical';
	return engine !== 'mongodb' || b.databases === 'all';
}

export type Snapshot = {
	id: string; chain: string; type: 'full' | 'incremental'; server: string; engine: Engine; database: string;
	size: number; created: string; expires: string; status: Status; destinations: string[]; checksum: string;
};

const snap = (id: string, chain: string, type: Snapshot['type'], server: string, engine: Engine, database: string, size: number, hoursAgo: number, status: Status = 'completed'): Snapshot => ({
	id, chain, type, server, engine, database, size, status,
	created: ago(hoursAgo), expires: ago(hoursAgo - 24 * 90), destinations: ['minio-utama'],
	checksum: 'sha256:9f2c4b7e0a1d'
});

export const snapshots: Snapshot[] = [
	snap('sn-108', 'c1', 'incremental', 'prod-pg', 'postgresql', 'cluster', 0.09 * GB, 2),
	snap('sn-107', 'c1', 'incremental', 'prod-pg', 'postgresql', 'cluster', 0.14 * GB, 8),
	snap('sn-106', 'c2', 'incremental', 'prod-mysql', 'mysql', 'shop', 0, 9, 'failed'),
	snap('sn-105', 'c1', 'incremental', 'prod-pg', 'postgresql', 'cluster', 0.11 * GB, 14),
	snap('sn-104', 'c3', 'full', 'cache-redis', 'redis', 'all', 0.6 * GB, 20),
	snap('sn-103', 'c1', 'full', 'prod-pg', 'postgresql', 'cluster', 18.4 * GB, 26),
	snap('sn-102', 'c2', 'full', 'prod-mysql', 'mysql', 'shop', 4.2 * GB, 33),
	snap('sn-101', 'c4', 'full', 'crm-mongo', 'mongodb', 'all', 7.9 * GB, 80),
	snap('sn-100', 'c5', 'full', 'prod-pg', 'postgresql', 'cluster', 17.9 * GB, 24 * 9),
	snap('sn-099', 'c6', 'full', 'prod-mysql', 'mysql', 'shop', 4.0 * GB, 24 * 21),
	snap('sn-098', 'c7', 'full', 'crm-mongo', 'mongodb', 'all', 7.1 * GB, 24 * 45)
];

export type Job = {
	id: string; type: 'backup' | 'restore' | 'delete' | 'test'; target: string; agent: string;
	status: Status; progress: number; started: string; duration: string;
};

export const jobs: Job[] = [
	{ id: 'j-2041', type: 'backup', target: 'prod-pg', agent: 'local', status: 'running', progress: 68, started: ago(0.05), duration: '3m 02s' },
	{ id: 'j-2040', type: 'backup', target: 'prod-mysql', agent: 'local', status: 'failed', progress: 0, started: ago(9), duration: '31s' },
	{ id: 'j-2039', type: 'backup', target: 'prod-pg', agent: 'local', status: 'completed', progress: 100, started: ago(14), duration: '48s' },
	{ id: 'j-2038', type: 'restore', target: 'staging-pg', agent: 'local', status: 'completed', progress: 100, started: ago(18), duration: '12m 40s' },
	{ id: 'j-2037', type: 'backup', target: 'cache-redis', agent: 'local', status: 'completed', progress: 100, started: ago(20), duration: '1m 12s' },
	{ id: 'j-2036', type: 'delete', target: 'minio-utama', agent: 'local', status: 'pending', progress: 0, started: ago(0.01), duration: '-' },
	// Older history so the date filters have something to filter.
	...Array.from({ length: 12 }, (_, i): Job => ({
		id: `j-${2035 - i}`, type: i % 5 === 4 ? 'restore' : 'backup', target: servers[i % servers.length].name, agent: 'local',
		status: i === 3 ? 'failed' : 'completed', progress: i === 3 ? 0 : 100, started: ago(30 + i * i * 7), duration: `${(i % 4) + 1}m ${10 + i}s`
	}))
];

export const jobLog = (job: Job): string[] => [
	`[info] job ${job.id} claimed by agent ${job.agent}`,
	'[info] opening SSH tunnel to bastion.example.com:22',
	'[info] connection ok, server version detected',
	'[info] dump started, compression zstd level 3',
	...(job.status === 'failed'
		? ["[error] Access denied for user 'backup'@'10.0.0.5' (using password: YES)", '[error] dump exited with status 2, upload aborted']
		: ['[info] uploaded 12.4 GB to minio-utama', ...(job.status === 'completed' ? ['[ok] checksum recorded, job completed'] : [])])
];

/** Last 60 job results, oldest first. */
export const jobHistory: { id: string; status: Status; target: string; started: string }[] = Array.from({ length: 60 }, (_, i) => ({
	id: `j-${1982 + i}`,
	status: [11, 27, 28, 46, 58].includes(i) ? 'failed' : i === 59 ? 'running' : i === 57 ? 'pending' : 'completed',
	target: servers[i % servers.length].name,
	started: ago((60 - i) * 2.4)
}));

/** Successful and failed backups per day, oldest first. */
export const activity = [
	[11, 0], [12, 0], [12, 1], [11, 0], [13, 0], [9, 0], [9, 0],
	[12, 0], [12, 0], [10, 2], [13, 0], [12, 0], [12, 0], [8, 1]
];

export const channels = [
	{ id: 'n1', name: 'Tim DBA', type: 'Telegram', target: 'chat -100123456', events: 4 },
	{ id: 'n2', name: 'Ops', type: 'Mattermost', target: '#ops-alerts', events: 2 }
];
