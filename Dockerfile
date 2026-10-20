# syntax=docker/dockerfile:1

FROM node:22-alpine AS web
WORKDIR /app/web
COPY web/package.json web/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY web/ ./
RUN npm run build

FROM rust:1-slim-bookworm AS rust
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --release --locked

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl tzdata \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home sfos

WORKDIR /app
COPY --from=rust /src/target/release/sfos-api /usr/local/bin/sfos-api
COPY --from=rust /src/target/release/sfos-ingest /usr/local/bin/sfos-ingest
COPY --from=web /app/web/dist /app/web/dist

ENV WEB_DIR=/app/web/dist
USER sfos
EXPOSE 8080 514/tcp 514/udp 8081

CMD ["sfos-api"]
