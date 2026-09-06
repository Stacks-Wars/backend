# Solana play fixture

Public play key for local `cargo run` / `bun run dev`. Not the wars key.

- mnemonic: `endless core tobacco mechanic have kiwi act issue page custom friend room`
- pubkey: `BQXTZq9YdAwCCNG1X4kx19XjFVBvkPYt6z8NQVQZejFm`
- mint: `5K2oEy9qR3g4ZRKmR6ANcr812eoig6t3zb62YULVEDfY`
- program: `9tbiacbM3R4z6hubRujtKfvPn5yNgrvwUB8oEvK6cBfL`
- derivation: `m/44'/501'/0'/0'`

`--main` keeps the wars mint `2ztYAL…` / program `8NZHj9…`.

Create the mint, deploy the program, and `initialize` with:

```bash
cd frontend && node ../backend/sw-vault/solana/scripts/deploy-play.mjs
cd frontend && SOLANA_KEY="$(cat ../backend/sw-vault/solana/play/mnemonic.txt)" \
  SOLANA_USDC_MINT=5K2oEy9qR3g4ZRKmR6ANcr812eoig6t3zb62YULVEDfY \
  SOLANA_VAULT_PROGRAM_ID=9tbiacbM3R4z6hubRujtKfvPn5yNgrvwUB8oEvK6cBfL \
  node scripts/initialize-solana-vault.mjs
```
