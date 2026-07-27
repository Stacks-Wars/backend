# Stacks Wars Backend

Standalone backend foundation for **Stacks Wars** — a multiplayer gaming platform on the Stacks blockchain.

Players will create or join lobbies for a specific game, optionally stake STX/FT (or join sponsored games), play in real time, earn wars points / rankings across seasons, and claim prizes on-chain.

Clients (web / mobile) live in a separate monorepo. This repository does **not** depend on those packages.

> This tree is a **shell**: wiring, contracts, and empty module boundaries. Domain logic is intentionally unfinished.

## Architecture

| Crate | Purpose |
| --- | --- |
| `sw-domain` | Shared domain types (`User`, `Lobby`, lifecycle enums, seasons, …) |
| `sw-plugin` | Portable game plugin contract (`GameEngine`, `GameFactory`, `GameHost`, registry) |
| `sw-game-noop` | Trivial example game that proves registration works |
| `sw-server` | HTTP + WebSocket server binary (Axum / Tokio) |

**Stack**

- **Rust** + **Axum** (HTTP + WebSocket) on **Tokio**
- **Postgres** via **SQLx** for durable data (required at boot)
- **Redis** for runtime / lobby state (required at boot)
- **Neon Auth** on the frontend owns end-user sessions; this API uses `INTERNAL_API_SECRET` for trusted user sync

`DATABASE_URL`, `REDIS_URL`, and `INTERNAL_API_SECRET` are required. The process exits on missing config or failed connect/ping.

SQL migrations live in `migrations/` (applied to the Neon `dev` branch). App users are upserted via `POST /users` (requires `x-internal-secret`). Custodial wallets live under `GET|POST /users/{id}/custodial-wallet` and are separate from `users.wallet_address` (personal rewards payout address, linked later).

```
clients ──HTTP/WS──► sw-server ──hosts──► GameEngine (from plugin crates)
                         │                      │
                         │                      └── calls GameHost (broadcast, save, finish)
                         ├── Postgres (users, seasons, history)   [connected; queries later]
                         └── Redis (lobby runtime)                [connected; state later]
```

## Game plugin system

Games are **not** built inside the server. Each game is a separate crate that depends on `sw-plugin` (and optionally `sw-domain`), never on `sw-server`.

Contract surface:

- **`GameEngine`** — per-lobby runtime the server can host
- **`GameFactory`** — constructs an engine for a lobby + exposes catalog metadata
- **`GameHost`** — platform capabilities engines may call (broadcast, send_to, checkpoint, complete_match)
- **`GameRegistry`** — in-process `game_id → factory` map

### Adding a game later

1. Publish a crate that implements `GameFactory` / `GameEngine`.
2. Depend on it from `sw-server` (or load via a registration module).
3. Register the factory at boot:

```rust
games.register(MyGameFactory::arc())?;
```

The shell registers `sw-game-noop` (`game_id = "noop"`) so catalog endpoints return a real entry.

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
# set DATABASE_URL, REDIS_URL, INTERNAL_API_SECRET
cargo run -p sw-server
```

Useful endpoints:

| Method | Path | Notes |
| --- | --- | --- |
| `GET` | `/health` | Live check against Postgres + Redis + plugin counts |
| `GET` | `/games` | Catalog from the plugin registry |
| `GET` | `/games/{game_id}` | Single registered game |
| `GET` | `/ws` | WebSocket shell (welcome + echo) |
| `*` | `/auth`, `/lobbies`, … | Stubs → `501 not_implemented` |

## What is intentionally unfinished

- Lobby create/join/matchmaking, chat, countdown, prize claim
- Redis lobby state machines
- Seasons, wars points aggregation, leaderboards
- On-chain balance / join / claim verification
- Neon JWT verification on protected API routes (identity is Neon Auth on the frontend today)
- Actual game rules (beyond the no-op plugin)

Grow the platform domain-by-domain against these boundaries.
