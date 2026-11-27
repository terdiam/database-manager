// Shared constants, and sample data for the page that has no backend yet (SSH tunnels).
export type Engine = 'postgresql' | 'mysql' | 'mariadb' | 'mongodb' | 'sqlite' | 'redis' | 'opensearch' | 'elasticsearch';
export type Status = 'completed' | 'running' | 'failed' | 'pending' | 'cancelled' | 'online' | 'offline';

/** Engines that work today; the others are shown as coming. */
export const supported: Engine[] = ['postgresql', 'mysql', 'mariadb', 'mongodb', 'sqlite'];

/** The values sealed for an agent. SQLite has only a folder below the agent's root. */
export function connectionOf(engine: Engine, form: { host: string; port: number; username: string; password: string; ssl_mode: string }) {
	return engine === 'sqlite'
		? { host: form.host.trim(), port: 0, username: '', password: '', ssl_mode: 'disable' }
		: { host: form.host.trim(), port: form.port, username: form.username.trim(), password: form.password, ssl_mode: form.ssl_mode };
}

/** Allowed database names; for SQLite a file name without a path. */
export const namePattern = (engine?: Engine) => (engine === 'sqlite' ? '[A-Za-z0-9_][A-Za-z0-9_.\\-]*' : '[A-Za-z0-9_][A-Za-z0-9_\\-]*');

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

export const sshConfigs = [
	{ id: 's1', name: 'bastion-prod', host: 'bastion.example.com', port: 22, user: 'backup', auth: 'key', used: 2 },
	{ id: 's2', name: 'jump-staging', host: '10.8.0.4', port: 2222, user: 'deploy', auth: 'password', used: 1 }
];
