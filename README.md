# DB Manager

Self-hosted database backup and restore with a web dashboard.
Rust (server and agent), PostgreSQL and SvelteKit.

Work in progress: accounts and agents work, backups are not built yet. Pages marked "sample data" show dummy content.

## Features

Done:

- [x] Login with roles (admin, operator, viewer)
- [x] Audit log
- [x] Agents with one-time enrollment
- [x] Encrypted API payloads, can be turned off
- [x] Encryption for sensitive data in the database
- [x] Optional Cloudflare Turnstile on login
- [x] HTTPS with Hanoey
- [x] Indonesian and English UI, dark mode
- [x] Dashboard and backup screens (UI only)

Planned:

- [ ] Backup and restore for PostgreSQL to S3/MinIO
- [ ] Schedules and retention
- [ ] Live monitoring and Telegram/Mattermost alerts
- [ ] MySQL, MariaDB, MongoDB, SQLite
- [ ] SSH tunnel; SFTP, FTP, Samba and Google Drive storage
- [ ] Redis, Elasticsearch, OpenSearch
- [ ] Compressed and encrypted backup files
- [ ] Incremental backups

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
DBM_ENROLL_TOKEN=<token> docker compose --profile agent up -d agent
```

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
