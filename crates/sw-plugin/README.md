# sw-plugin

Portable game plugin contract for **Stacks Wars**.

Implement `GameFactory` / `GameEngine` and call into `GameHost` for broadcast, persistence hooks, and match completion. Game crates should depend on `sw-plugin` + `sw-domain` — never on `sw-server`.

Winner-take-all is the default: call `complete_match` with a named winner and the host issues one full-pot claim. To split a pot (or pay as ranks lock), call `issue_payout` yourself — it is a no-op on older hosts. Optional helpers `placement_share_pct` / `placement_prize` implement the first-party 70/30 and 50/30/20 splits; other games can ignore them. Finish those matches with `stats.settlement = "distributed"` so settle does not issue a second winner-take-all claim.

Match Wars Points: pass the engine winner flag into `save_player_result` / `calculate_wars_point_for` so draws do not get the win bonus. `calculate_wars_point` still treats rank 1 as a win for host-save fallbacks.

```toml
sw-plugin = "1.0.4"
sw-domain = "1.0.2"
```
