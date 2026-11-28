FROM node:22-bookworm-slim AS web
WORKDIR /web
RUN corepack enable
COPY web/package.json web/pnpm-lock.yaml web/pnpm-workspace.yam[l] web/.npmrc ./
RUN corepack install -g pnpm@12 && pnpm install --frozen-lockfile
COPY web/ .
RUN pnpm build

FROM rust:1-bookworm AS build
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
COPY --from=web /web/build web/build
RUN cargo build --release --locked

FROM debian:bookworm-slim AS base
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
RUN useradd --system --uid 10001 dbm

FROM base AS server
COPY --from=build /app/target/release/dbm-server /usr/local/bin/
USER dbm
ENV DBM_LISTEN=0.0.0.0:8080
EXPOSE 8080
CMD ["dbm-server"]

# Client tools come from each vendor's own image.
FROM mysql:8.4 AS mysql-tools
# The server image carries no mysqlbinlog: the full client comes from the same signed repository.
RUN printf '[mysql-client]\nname=MySQL client\nbaseurl=https://repo.mysql.com/yum/mysql-8.4-community/el/9/$basearch/\ngpgcheck=1\ngpgkey=file:///etc/pki/rpm-gpg/RPM-GPG-KEY-mysql\nmodule_hotfixes=true\n' > /etc/yum.repos.d/mysql-client.repo \
    && microdnf remove -y mysql-community-server-minimal mysql-shell \
    && microdnf install -y mysql-community-client
FROM mongo:8.0 AS mongo-tools
# Same Debian release as the agent image; the newest Redis 8 is only built for it.
FROM redis:8-trixie AS redis-tools

# Combining a PostgreSQL incremental needs the tool of the server's own major version.
FROM postgres:19-trixie AS postgres-tools
RUN mkdir -p /etc/postgresql-common && echo 'create_main_cluster = false' > /etc/postgresql-common/createcluster.conf \
    && apt-get update && apt-get install -y --no-install-recommends postgresql-17 postgresql-18

# The agent needs the database client tools: a dump tool must be at least as new as the
# databases it backs up.
FROM postgres:19-trixie AS agent
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates mariadb-client sqlite3 libncurses6 libgssapi-krb5-2 openssh-client smbclient \
    && rm -rf /var/lib/apt/lists/*
# MySQL's own tools in /usr/local/bin come before MariaDB's compatibility names.
COPY --from=mysql-tools /usr/bin/mysql /usr/bin/mysqldump /usr/bin/mysqlbinlog /usr/local/bin/
COPY --from=mongo-tools /usr/bin/mongodump /usr/bin/mongorestore /usr/local/bin/
COPY --from=postgres-tools /usr/lib/postgresql/17/bin/pg_combinebackup /usr/lib/postgresql/17/bin/
COPY --from=postgres-tools /usr/lib/postgresql/18/bin/pg_combinebackup /usr/lib/postgresql/18/bin/
# redis-server loads a snapshot during a restore; redis-cli takes one.
COPY --from=redis-tools /usr/local/bin/redis-server /usr/local/bin/redis-cli /usr/local/bin/
# Fails the build if a copied tool cannot start here.
RUN mysql --version && mysqldump --version && mysqlbinlog --version && mariadb-binlog --version && mongodump --version && mongorestore --version \
    && mariadb-dump --version && sqlite3 --version && ssh -V && smbclient --version \
    && redis-server --version && redis-cli --version \
    && for v in 17 18 19; do /usr/lib/postgresql/$v/bin/pg_combinebackup --version; done
RUN useradd --system --uid 10001 --create-home dbm
COPY --from=build /app/target/release/dbm-agent /usr/local/bin/
RUN mkdir /data /sqlite /pgdata && chown dbm /data /sqlite /pgdata
USER dbm
ENV DBM_STATE_FILE=/data/agent-state
ENTRYPOINT []
CMD ["dbm-agent"]
