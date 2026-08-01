# AGENT.md — Stacks Wars Backend

Canonical docs: https://docs.stackswars.com/

Do not invent architecture. Prefer docs + `sw-domain` / `sw-plugin` contracts.

## Workspace

| Crate | Owns |
|-------|------|
| `sw-domain` | Shared IDs, lobby/game/accounting DTOs |
| `sw-plugin` | `GameFactory`, `GameEngine`, `GameHost`, registry, kit |
| `sw-server` | Axum HTTP + WS, persistence, vault verification, host impl |

Game engines live in separate crates (crates.io `sw_*`), not inside `sw-server` sources. Register them in `crates/sw-server/src/games.rs`.

## Boundaries

1. **Domain ownership** — types and invariants belong in `sw-domain`. Servers/plugins consume them; do not fork duplicate structs in routes.
2. **Plugin purity** — game crates talk to the platform only through `GameHost`. No SQL, Redis, Hiro, or vault calls inside games.
3. **One plugin identity** — do not mix path and crates.io `sw-plugin` in one build (duplicate traits). Align versions intentionally.
4. **Event-driven rooms** — engines push state via host broadcast/send; HTTP is for auth, lobby setup, wallet, and claims — not per-move gameplay.

## WebSocket

- Multiplexed endpoint `/app` with topic subscriptions.
- Game actions → `GameEngine::handle_action`.
- Preserve existing message shapes (`camelCase`) unless coordinating a versioned break.

## Transactions / vault

- Paid lobbies: verify on-chain vault txs before seating.
- Settle via `complete_match` → claim intents → client vault claim; do not invent a second payout path.
- Respect fee/entry limits in server config.

## Database

- Schema changes = new migrations under `backend/migrations/`.
- Use existing SQLx patterns; no ad-hoc dual ORMs.
- Redis is for seats, cache, short-lived coordination — not source of truth for balances (Hiro FT is).

## Games

- Implement `GameFactory` + `GameEngine`; set production `dev_id` from the developer’s platform user id.
- Register in `games.rs` and depend on the published crate version for deploy builds.
- Details: https://docs.stackswars.com/develop/interfaces

## Don’t

- Publish crates under MIT; use the Stacks Wars Custom License for platform/game code.
- Commit `.env` secrets or oracle mnemonics into the repo.
