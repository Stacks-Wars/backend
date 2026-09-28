# Stacks Wars Backend

Standalone backend for **Stacks Wars** — multiplayer arenas with custodial USDCx balances, live lobbies, seasons, and pluggable game engines.

Clients live in a separate frontend. This repository does **not** depend on those packages.

## Architecture

| Crate       | Purpose                                                                           |
| ----------- | --------------------------------------------------------------------------------- |
| `sw-domain` | Shared domain types (`User`, `Lobby`, seasons, wallet DTOs, …)                    |
| `sw-plugin` | Portable game plugin contract (`GameEngine`, `GameFactory`, `GameHost`, registry) |
| `sw-server` | HTTP + WebSocket server binary (Axum / Tokio)                                     |
| `sw-cron`   | Scheduled one-shot jobs (seasons, quest nudge, free-lobby TTL)                    |

**Stack**

- **Rust** + **Axum** (HTTP + WebSocket) on **Tokio**
- **Postgres** via **SQLx** for durable data (required at boot)
- **Redis** for lobby runtime state (required at boot)
- **Better Auth** on the frontend owns end-user sessions; this API verifies those JWTs (JWKS) on user and admin routes

`cargo run -p sw-server` is local/dev and needs no `.env` (Compose Postgres + Redis). Production is `--main` / `NETWORK=main`, which requires `DATABASE_URL`, `REDIS_URL`, `HIRO_API_KEY`, `APP_URL`, `INTERNAL_API_SECRET`, and `HELIUS_API_KEY`. `APP_URL` is the site origin (JWKS + deep links).

Scheduled jobs run from the same image as `sw-cron` (see [Scheduled jobs](#scheduled-jobs-sw-cron)). The API process itself schedules nothing; it only serves.

SQL migrations live in `migrations/`. App users are upserted via `POST /users` (Bearer JWT; `id` = Better Auth `sub`). Custodial wallets live under `GET|POST /users/{id}/custodial-wallet`. Platform balances use `/wallet` (each chain's official explorer / RPC + Redis cache, chain activity, withdrawals) — vault escrow is on-chain. Admin season routes require a verified JWT email on the `ADMIN` allowlist.

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
docker compose up -d postgres redis
cargo run -p sw-server
```

## Docker / Railway

Build context is this `backend/` directory. One image ships **two** binaries:
`sw-server` (the API, default `CMD`) and `sw-cron` (scheduled jobs). Railway runs
the API as one service and each cron job as its own service on the same image.

```bash
docker build -t sw-server .
docker run --rm -p 8080:8080 --env-file .env sw-server
docker run --rm --env-file .env sw-server /app/sw-cron season
```

Railway uses `railway.toml` (`builder = DOCKERFILE`).

`HOST`, `PORT`, and `MIGRATIONS_DIR` are set in the image. Railway may still inject `PORT`. Health check: `GET /health`.

## Scheduled jobs (`sw-cron`)

```bash
cargo run -p sw-cron -- all              # every due job, then exit (what Railway runs)
cargo run -p sw-cron -- season           # just one job, for local work
cargo run -p sw-cron -- all --loop       # keep ticking hourly instead of exiting
cargo run -p sw-cron -- --help           # list jobs
```

One service, one schedule: `0 * * * *`. Each job gates itself, so the hourly tick
covers all three cadences.

| Job | Due when |
| --- | --- |
| `season` | the current season ends within 6h and its successor does not exist |
| `quest-nudge` | UTC hour is 10–21 and today's send slots are unclaimed |
| `lobby-free-ttl` | a free waiting lobby is older than 24h |

`all` builds one `AppState` and shares it across the jobs. A failing job is
logged, the rest still run, and the process exits non-zero so Railway records a
failed run.

### Setting the service up on Railway

1. Merge and let the existing **sw-server** service redeploy. Its config is
   unchanged; the image now contains a second binary.
2. **New Service → Deploy from a GitHub repo** → same repo and branch as the
   server. (One extra service; the API is untouched.)
3. On that service set:
   - **Settings → Deploy → Custom Start Command**: `/app/sw-cron all`
   - **Settings → Deploy → Cron Schedule**: `0 * * * *`
   - **Variables**: `DATABASE_URL`, `REDIS_URL`, `APP_URL`,
     `INTERNAL_API_SECRET`, `HIRO_API_KEY`, `HELIUS_API_KEY`,
     `VAPID_PUBLIC_KEY` and `VAPID_PRIVATE_KEY`. Reference the server's values
     with `${{sw-server.VARIABLE_NAME}}` instead of copying secrets by hand.
4. **Do not set a healthcheck path.** The process exits after its work, so
   nothing serves `/health` and the deploy would be marked failed.

`NETWORK=main` is baked into the image, which is what keeps these jobs in main
mode; a per-service start command replaces the server's `CMD`, so the `--main`
flag on that `CMD` never runs for them.

The binary runs migrations on connect, so the first run applies any pending
migration; no separate migration step is needed.

> Config as Code (`railway.toml`) is deprecated and stops being read on
> 2026-12-01. New services cannot opt into it, so the settings above live in the
> dashboard; the healthcheck and restart policy currently in `railway.toml` will
> need to move to the dashboard (or to Infrastructure as Code) before then.

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
