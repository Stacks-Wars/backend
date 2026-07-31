# sw-plugin

Portable game plugin contract for **Stacks Wars**.

Implement `GameFactory` / `GameEngine` and call into `GameHost` for broadcast, persistence hooks, and match completion. Game crates should depend on `sw-plugin` + `sw-domain` — never on `sw-server`.

```toml
sw-plugin = "1.0.0"
sw-domain = "1.0.0"
```
