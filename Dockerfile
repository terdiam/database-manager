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

FROM base AS agent
COPY --from=build /app/target/release/dbm-agent /usr/local/bin/
RUN mkdir /data && chown dbm /data
USER dbm
ENV DBM_STATE_FILE=/data/agent-state
CMD ["dbm-agent"]
