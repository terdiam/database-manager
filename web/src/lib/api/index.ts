// Shared constants about database engines and statuses.
export type Engine = 'postgresql' | 'mysql' | 'mariadb' | 'mongodb' | 'sqlite' | 'redis' | 'opensearch' | 'elasticsearch';
export type Status = 'completed' | 'running' | 'failed' | 'pending' | 'cancelled' | 'online' | 'offline';


/** The values sealed for an agent. SQLite has only a folder below the agent's root. */
export function connectionOf(engine: Engine, form: { host: string; port: number; username: string; password: string; ssl_mode: string }) {
	return engine === 'sqlite'
		? { host: form.host.trim(), port: 0, username: '', password: '', ssl_mode: 'disable' }
		: { host: form.host.trim(), port: form.port, username: form.username.trim(), password: form.password, ssl_mode: form.ssl_mode };
}

const search = (engine?: Engine) => engine === 'elasticsearch' || engine === 'opensearch';

/** What a backup or restore may name; mirrors the server's rules. */
export const namePattern = (engine?: Engine) =>
	engine === 'redis' ? 'all'
	: engine === 'sqlite' ? '[A-Za-z0-9_][A-Za-z0-9_.\\-]*'
	: search(engine) ? '_all|[a-z0-9][a-z0-9_.\\-]*'
	: '[A-Za-z0-9_][A-Za-z0-9_\\-]*';

/** Label and help of the field that holds that name. */
export const targetField = (engine?: Engine) =>
	engine === 'sqlite' ? ({ label: 'field.fileName', help: 'help.fileName' } as const)
	: engine === 'redis' ? ({ label: 'field.databaseName', help: 'help.redisAll' } as const)
	: search(engine) ? ({ label: 'field.indexName', help: 'help.indexName' } as const)
	: engine === 'mongodb' ? ({ label: 'field.databaseName', help: 'help.mongoDatabase' } as const)
	: engine === 'postgresql' ? ({ label: 'field.databaseName', help: 'help.pgDatabase' } as const)
	: ({ label: 'field.databaseName', help: 'help.databaseName' } as const);

/** Whether incremental backups can follow a full one; mirrors the server's rule. */
export const canFollow = (engine: Engine, database: string) =>
	engines[engine].incremental && ((engine !== 'mongodb' && engine !== 'postgresql') || database === '_all');

/** TLS choices. Only the SQL servers can fall back to plain text. */
export const tlsModes = (engine: Engine) =>
	engine === 'mongodb' || engine === 'redis' || search(engine) ? ['disable', 'require', 'verify-full'] : ['disable', 'prefer', 'require', 'verify-full'];

/** Servers that may run without any login, or with a password only. */
export const loginOptional = (engine: Engine) => engine === 'mongodb' || engine === 'redis' || search(engine);

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
