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
- **JWT** helpers / extractors as auth boundaries (no real login flow yet)

`DATABASE_URL`, `REDIS_URL`, and `JWT_SECRET` are required. The process exits on missing config or failed connect/ping.

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

## Run

Requirements: Rust stable (edition 2024 / recent toolchain).

```bash
cp .env.example .env
# set DATABASE_URL, REDIS_URL, JWT_SECRET
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
- SQL schemas / queries and Redis lobby state machines
- Seasons, wars points aggregation, leaderboards
- On-chain balance / join / claim verification
- Real auth (wallet login, token issuance, admin authorization)
- Actual game rules (beyond the no-op plugin)

Grow the platform domain-by-domain against these boundaries.
