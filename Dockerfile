# ---- web UI ----------------------------------------------------------------
FROM node:22-alpine AS web
WORKDIR /app
RUN corepack enable
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml ./
RUN pnpm install --frozen-lockfile --ignore-scripts
COPY web ./web
COPY shared ./shared
RUN pnpm build:web

# ---- binary ----------------------------------------------------------------
FROM rust:1-slim AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock build.rs ./
COPY src ./src
COPY --from=web /app/web/dist ./web/dist
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config libssl-dev \
 && cargo build --release \
 && strip target/release/proxyhand

FROM debian:stable-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/proxyhand /usr/local/bin/proxyhand
VOLUME ["/data"]
EXPOSE 7747
ENTRYPOINT ["proxyhand", "--port", "7747", "--host", "0.0.0.0", "--dir", "/data", "--self-hosts", "proxy"]
