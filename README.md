# Stacks Wars Backend

Standalone backend for **Stacks Wars** — multiplayer arenas with custodial USDCx balances, live lobbies, seasons, and pluggable game engines.

Clients live in a separate frontend. This repository does **not** depend on those packages.

## Architecture

| Crate       | Purpose                                                                           |
| ----------- | --------------------------------------------------------------------------------- |
| `sw-domain` | Shared domain types (`User`, `Lobby`, seasons, wallet DTOs, …)                    |
| `sw-plugin` | Portable game plugin contract (`GameEngine`, `GameFactory`, `GameHost`, registry) |
| `sw-server` | HTTP + WebSocket server binary (Axum / Tokio)                                     |

**Stack**

- **Rust** + **Axum** (HTTP + WebSocket) on **Tokio**
- **Postgres** via **SQLx** for durable data (required at boot)
- **Redis** for lobby runtime state (required at boot)
- **Better Auth** on the frontend owns end-user sessions; this API verifies those JWTs (JWKS) on user and admin routes

`DATABASE_URL`, `REDIS_URL`, `HIRO_API_KEY`, `BETTER_AUTH_URL`, `SW_VAULT_CONTRACT`, and `INTERNAL_API_SECRET` are required. The process exits on missing config or failed connect/ping. Use the same `INTERNAL_API_SECRET` in the Next.js app for server-to-server calls.

Optional Telegram companion: set both `TELEGRAM_BOT_TOKEN` and `TELEGRAM_CHAT_ID` to announce public lobbies, post match results, and serve `/leaderboard`. `FRONTEND_URL` controls deep links (default `https://stackswars.com`).

SQL migrations live in `migrations/`. App users are upserted via `POST /users` (Bearer JWT; `id` = Better Auth `sub`). Custodial wallets live under `GET|POST /users/{id}/custodial-wallet`. Platform balances use `/wallet` (each chain's official explorer / RPC + Redis cache, chain activity, withdrawals) — vault escrow is on-chain via `SW_VAULT_CONTRACT`. Admin season routes require a verified JWT email on the `ADMIN` allowlist.

```
clients ──HTTP/WS──► sw-server ──hosts──► GameEngine (from plugin crates)
                         │                      │
                         │                      └── calls GameHost (broadcast, save, finish)
                         ├── Postgres (users, lobbies, seasons)
                         ├── Redis (lobby runtime, balance cache)
                         └── chain explorers / RPC (balance, tx status)
```

## Game plugin system

Games are **not** built inside the server. Each game is a separate crate that depends on `sw-plugin` (and optionally `sw-domain`), never on `sw-server`.

Contract surface:

- **`GameEngine`** — per-lobby runtime the server can host
- **`GameFactory`** — constructs an engine for a lobby + exposes catalog metadata
- **`GameHost`** — platform capabilities engines may call (broadcast, send_to, checkpoint, complete_match)
- **`GameRegistry`** — in-process `game_id → factory` map

### Adding a game

1. Publish a crate that implements `GameFactory` / `GameEngine`.
2. Depend on it from `sw-server` and register in `games.rs`.
3. Register the factory at boot:

```rust
games.register(MyGameFactory::arc())?;
```

## Migrations

SQL lives in `migrations/` (SQLx format). They also run automatically when the server boots.

```bash
# one-time: install the CLI
cargo install sqlx-cli --no-default-features --features rustls,postgres

# from the backend repo root (reads DATABASE_URL from .env)
cargo migrate          # alias for: cargo sqlx migrate run --source migrations
cargo migrate-info     # show applied / pending
cargo migrate-add name # create a new reversible migration pair
```

Or without the alias:

```bash
cargo sqlx migrate run --source migrations
```

## Run

Requirements: Rust stable (edition 2024 / recent toolchain).

```bash
cp .env.example .env
# set DATABASE_URL, REDIS_URL, HIRO_API_KEY, BETTER_AUTH_URL, SW_VAULT_CONTRACT, INTERNAL_API_SECRET
cargo run -p sw-server
```

## Docker / Railway

Build context is this `backend/` directory. Railway should run **sw-server** (Dockerfile) + a **Redis** plugin. Postgres can stay on Neon (`DATABASE_URL`).

```bash
docker build -t sw-server .
docker run --rm -p 8080:8080 --env-file .env -e MIGRATIONS_DIR=/app/migrations sw-server
```

Railway uses `railway.toml` (`builder = DOCKERFILE`). Set at least:

| Variable              | Notes                         |
| --------------------- | ----------------------------- |
| `DATABASE_URL`        | Neon (or Railway Postgres)    |
| `REDIS_URL`           | from Railway Redis            |
| `HIRO_API_KEY`        |                               |
| `BETTER_AUTH_URL`     | Next.js origin (JWKS source)  |
| `SW_VAULT_CONTRACT`   |                               |
| `INTERNAL_API_SECRET` | same value as the Next.js app |
| `FRONTEND_URL`        | production site origin        |
| `PORT`                | Railway injects this          |
| `TELEGRAM_*`          | optional                      |

`MIGRATIONS_DIR` defaults to `/app/migrations` in the image. Health check: `GET /health`.

Useful endpoints:

| Method       | Path                       | Notes                                               |
| ------------ | -------------------------- | --------------------------------------------------- |
| `GET`        | `/health`                  | Live check against Postgres + Redis + plugin counts |
| `GET`        | `/games`                   | Catalog from the plugin registry                    |
| `GET`        | `/games/{game_id}`         | Single registered game                              |
| `GET`/`POST` | `/lobbies…`                | Create, join, ready, start; micro-USDCx entry       |
| `GET`/`POST` | `/wallet…`                 | On-chain balance, refresh, activity, withdrawals    |
| `GET`        | `/seasons`, `/leaderboard` | Season board                                        |
| `POST`/`PUT` | `/admin/seasons…`          | Admin JWT + verified allowlisted email              |
| `GET`        | `/app`                     | Multiplexed app WebSocket                           |
