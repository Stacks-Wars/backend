# Backend

Canonical docs: https://docs.stackswars.com/

Do not invent architecture. Prefer docs plus the `sw-domain` / `sw-plugin` contracts that already exist.

## Workspace

| Crate | Owns |
|-------|------|
| `sw-domain` | Shared IDs, lobby/game/accounting DTOs |
| `sw-plugin` | `GameFactory`, `GameEngine`, `GameHost`, registry, kit |
| `sw-server` | Axum HTTP + WS, persistence, vault verification, host impl |

Game engines live in separate crates (crates.io `sw_*`), not inside `sw-server` sources. Register them in `crates/sw-server/src/games.rs`.

## Boundaries

1. Types and invariants belong in `sw-domain`. Routes and plugins consume them. Do not fork duplicate structs in handlers.
2. Game crates talk to the platform only through `GameHost`. No SQL, Redis, Hiro, or vault calls inside games.
3. Do not mix a path dependency and a crates.io `sw-plugin` in one build. That duplicates traits. Align versions on purpose.
4. Engines push state through host broadcast/send. HTTP is for auth, lobby setup, wallet, and claims, not per-move gameplay.

## Rust

- `clippy` clean on the code you touch. No `unwrap` / `expect` in request handlers.
- Axum extractors that only exist to run auth (for example `InternalSecret`) may be bound as `_secret`. The underscore means the value is unused, not that auth is skipped. `FromRequestParts` still runs.
- Keep handler functions boring: extract, call a service, return JSON. Business rules go in `services/` or `data/`.
- Logging is tracing with structured fields (`lobby_id`, `user_id`, `path`). Do not format those into the message string.
- Schema changes are new files under `backend/migrations/`. Use the existing SQLx patterns. No second ORM.
- Redis is seats, cache, and short-lived coordination. Individual official chain explorers are the source of truth for balances.

## WebSocket

- Multiplexed endpoint `/app` with topic subscriptions.
- Game actions go to `GameEngine::handle_action`.
- Preserve existing `camelCase` message shapes unless you are coordinating a versioned break.

## Vault and settlement

- Paid lobbies: verify on-chain vault txs before seating.
- Settle via `complete_match` → claim intents → client vault claim. Do not add a second payout path. Games that split the pot call `issue_payout` as ranks lock (same claim-intent shape); `complete_match` with `stats.settlement = "distributed"` must not issue a second winner-take-all claim. Empty `winners` without that flag is still a draw refund.
- Respect fee and entry limits in server config.
- Chain-specific code (Hiro, Stacks principals, vault function names) stays behind the host / a chain adapter. Games and HTTP routes should not grow a second chain's types inline.

## Solana MCP

For Solana-related work, prefer the Solana Developer MCP tools over model memory.

Use `list_sections` first for non-trivial Solana questions so you can find the
right documentation source ids and section ids.

Use `get_documentation` when you need canonical docs for a specific source,
framework, library, or ecosystem area. Use `Solana_Documentation_Search` or
`Solana_Expert__Ask_For_Help` for narrow how-to questions, errors, or API usage.

Whenever you write or modify Solana program Rust, call `program_autofixer` before
returning code. It accepts `code`, optional `filename`, and optional `framework`
(`auto`, `anchor`, or `pinocchio`). Apply the suggested fixes, then call
`program_autofixer` again. Repeat until `require_another_tool_call_after_fixing`
is false.

## Janitors

- Free waiting lobbies older than 24h are expired by the Rust loop in `main.rs`.
- Paid waiting lobbies are refunded and expired by the Next cron (`/api/cron/lobby-ttl`), which calls `/admin/lobbies/*` with `x-internal-secret`. Do not duplicate that work in the Rust loop.
- Daily quest reminders: Next cron (`/api/cron/quest-nudge`, 10:00 UTC) calls `/admin/quests/daily-nudge`. Idempotent per user per UTC day.
- Admin internal routes authenticate with the `InternalSecret` extractor. The secret is `INTERNAL_API_SECRET`.

## Games

- Implement `GameFactory` + `GameEngine`. Production `dev_id` is the developer's platform user id.
- Register in `games.rs` and depend on the published crate version for deploy builds.
- Details: https://docs.stackswars.com/develop/interfaces

## Don't

- Publish crates under MIT. Platform and game code use the Stacks Wars Custom License.
- Commit `.env` secrets or oracle mnemonics.
- Let a missing game-dev wallet fail a winner claim. Missing dev wallet means 0% game fee and the platform principal as a placeholder.
