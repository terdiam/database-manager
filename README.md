# DB Manager

Self-hosted database backup and restore with a web dashboard.
Rust (server and agent), PostgreSQL and SvelteKit.

Work in progress: backup and restore work by hand or on a schedule. Pages marked "sample data" show dummy content.

## Features

Done:

- [x] Login with roles (admin, operator, viewer)
- [x] Audit log
- [x] Agents with one-time enrollment
- [x] PostgreSQL, MySQL, MariaDB, MongoDB and SQLite backup and restore to S3 or MinIO
- [x] Compressed and encrypted backup files
- [x] Live job logs
- [x] Encrypted API payloads, can be turned off
- [x] Encryption for sensitive data in the database
- [x] Optional Cloudflare Turnstile on login
- [x] HTTPS with Hanoey
- [x] Indonesian and English UI, dark mode
- [x] Schedules, and retention by days, months or GFS
- [x] Live dashboard
- [x] Telegram and Mattermost alerts
- [x] Download backup files
- [x] Edit databases, storage, backups, alerts, users and agent names
- [x] SSH tunnels to databases
- [x] SFTP, FTP/FTPS and Samba storage
- [x] Redis, Elasticsearch and OpenSearch
- [x] Incremental backups for MySQL, MariaDB, MongoDB and PostgreSQL

Planned:

- [ ] Google Drive storage

## Run

```bash
cp .env.example .env
```

Set `POSTGRES_PASSWORD` and `DBM_MASTER_KEY` (`openssl rand -base64 32`), then:

```bash
docker compose up -d --build
```

Open http://localhost:8080 and create the admin account right away.

Keep a copy of `DBM_MASTER_KEY`. Encrypted data cannot be recovered without it.

## Agent

Create a token on the Agents page, then:

```bash
DBM_ENROLL_TOKEN=<token> docker compose --profile agent up -d --build agent
```

The agent connects to your database and storage, so use addresses it can reach
(`host.docker.internal` for something running on the Docker host).

## SSH tunnels

Add the tunnel on the SSH Tunnel page, then choose it on a database. The database host and
port are as seen from the SSH server. The first host key seen is remembered; a changed key
stops the connection. Download links are only available for S3 destinations.

## SQLite

The agent only touches files inside its `DBM_SQLITE_ROOT` (`/sqlite` in Docker). Set
`DBM_SQLITE_DIR` to a host folder writable by uid 10001, or use the Docker volume. Add the
database with the folder below that root, then use the file name as the database name.
Stop the application that uses a file before restoring over it.

## Redis, Elasticsearch, OpenSearch

A Redis backup is the whole instance. Restoring replaces keys of the same name and leaves
the others; functions are not included, and the target should not be older than the source.

For Elasticsearch and OpenSearch use one index name, or `_all`. Restoring deletes and
recreates the index. Data streams, templates and users are not included.

## Incremental backups

For MySQL and MariaDB add a full backup, then an incremental one that follows it. The server
needs the binary log on in ROW format, and the user `RELOAD`, `REPLICATION SLAVE` and
`REPLICATION CLIENT` (MariaDB: `BINLOG MONITOR`). If the log was purged, a new full backup
is taken. A restore goes into a new or empty database.

For MongoDB the full backup takes the whole instance (database name `_all`) from a replica
set, and the incremental ones keep its oplog. Restoring writes every database back but
leaves the target's accounts alone; restore an incremental snapshot to get a single moment.

For PostgreSQL 17, 18 or 19 the full backup `_all` is a physical copy of the whole cluster.
The server needs `summarize_wal=on`, and the user the `REPLICATION` attribute with a
replication line in `pg_hba.conf`. Restoring writes a data directory into the agent's restore
folder (`DBM_PGDATA_DIR`, or the Docker volume); give it to the `postgres` user and start a
server of the same major version on it. Only one tablespace is supported. MySQL, MariaDB and
MongoDB can also restore to a point in time.

The MySQL and MariaDB incremental files hold the binary log of the whole server. Schema changes made across
databases while using the backed up one are replayed too.

## Backups

Add a database and a destination (S3, SFTP, FTP or Samba), then create a backup with a schedule or run it by hand.
A new retention setting applies to backups made after it. Passwords are
encrypted in the browser for the agent, which needs `https` or `localhost`.

Backup files are encrypted with [age](https://age-encryption.org). Download the keys from
Settings and keep them somewhere else: without them the files cannot be opened if the
server is lost.

## HTTPS

Self-signed certificate for local use:

```bash
mkdir -p certs && openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -days 365 -subj "/CN=localhost" -addext "subjectAltName=DNS:localhost,IP:127.0.0.1" -keyout certs/privkey.pem -out certs/fullchain.pem && chmod 644 certs/*.pem
```

Set `DBM_TRUST_PROXY=true`, then:

```bash
docker compose --profile https up -d
```

For a real domain, edit `deploy/hanoey.toml` and set `DBM_HTTPS_BIND=0.0.0.0`.

## Settings

All settings are in `.env`, see [.env.example](.env.example).

| Variable | Description |
|---|---|
| `DBM_E2EE` | `on` or `off`. Payload encryption on top of HTTPS |
| `DBM_TURNSTILE_SITE_KEY`, `DBM_TURNSTILE_SECRET_KEY` | Turnstile keys, leave empty to disable |
| `DBM_TURNSTILE_HOSTNAMES` | Allowed hostnames, for example `localhost` |
| `DBM_COOKIE_SECURE` | `false` only for plain HTTP on a trusted machine |
| `DBM_SQLITE_DIR` | Host folder with SQLite files for the agent |
| `DBM_PGDATA_DIR` | Host folder for PostgreSQL cluster restores |
| `DBM_ALLOW_UNSIGNED_RESTORE` | Agent: `true` to restore backups made before signing existed |

## Reset password

```bash
docker compose exec server dbm-server reset-password you@example.com
```

## Development

```bash
docker compose up -d db
set -a; . ./.env; set +a; cargo run -p dbm-server
```

```bash
pnpm --dir web install
pnpm --dir web dev
```

Tests:

```bash
cargo test
pnpm --dir web check && pnpm --dir web test
```
