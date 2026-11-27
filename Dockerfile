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
FROM mongo:8.0 AS mongo-tools

# The agent needs the database client tools: a dump tool must be at least as new as the
# databases it backs up.
FROM postgres:19-bookworm AS agent
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates mariadb-client sqlite3 libncurses6 libgssapi-krb5-2 openssh-client smbclient \
    && rm -rf /var/lib/apt/lists/*
# MySQL's own tools in /usr/local/bin come before MariaDB's compatibility names.
COPY --from=mysql-tools /usr/bin/mysql /usr/bin/mysqldump /usr/local/bin/
COPY --from=mongo-tools /usr/bin/mongodump /usr/bin/mongorestore /usr/local/bin/
# Fails the build if a copied tool cannot start here.
RUN mysql --version && mysqldump --version && mongodump --version && mongorestore --version \
    && mariadb-dump --version && sqlite3 --version && ssh -V && smbclient --version
RUN useradd --system --uid 10001 --create-home dbm
COPY --from=build /app/target/release/dbm-agent /usr/local/bin/
RUN mkdir /data /sqlite && chown dbm /data /sqlite
USER dbm
ENV DBM_STATE_FILE=/data/agent-state
ENTRYPOINT []
CMD ["dbm-agent"]
