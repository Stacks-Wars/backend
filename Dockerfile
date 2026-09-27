# syntax=docker/dockerfile:1.7
#
# Railway / container image for sw-server and sw-cron.
# Build context: this `backend/` directory.
#
# One image, two services:
#   - sw-server: the API + realtime server (default CMD, health check /health).
#   - sw-cron:   scheduled jobs, one per Railway cron service
#                (start command `/app/sw-cron <job>`), which must exit.
#
# Redis → Railway Redis plugin (REDIS_URL).
# Postgres → Neon or Railway Postgres (DATABASE_URL).
# Game plugins are detached until republished with real dev_ids.

FROM rust:1.94-bookworm AS builder

WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY migrations ./migrations

RUN cargo build --release -p sw-server -p sw-cron \
    && strip target/release/sw-server target/release/sw-cron

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /src/target/release/sw-server /app/sw-server
COPY --from=builder /src/target/release/sw-cron /app/sw-cron
COPY --from=builder /src/migrations /app/migrations

ENV HOST=0.0.0.0 \
    PORT=8080 \
    MIGRATIONS_DIR=/app/migrations \
    RUST_LOG=info,sw_server=info

EXPOSE 8080

USER nobody

CMD ["/app/sw-server", "--main"]
