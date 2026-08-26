# Solana vault

USDC escrow for paid Stacks Wars lobbies. Anchor 1.1. Players deposit USDC.
The platform key (`SOLANA_WARS_KEY`, path `m/44'/501'/0'/0'`) is:

- program deployer / upgrade authority
- fee payer and rent payer (players never need SOL)
- remaining signer on leave / kick / claim
- `Config.platform` — receives **2% of every claim on-chain**

This is not a Clarity port. State lives in PDAs + ATAs, not maps.

Program source: [`programs/src/lib.rs`](./programs/src/lib.rs).

## Accounts

- `Config` PDA (`["config"]`) — platform pubkey + USDC mint
- `LobbyEscrow` PDA (`["lobby", path_hash]`) — entry, pot, seats, claims-started
- `Seat` PDA (`["seat", path_hash, player]`) — that player's deposit
- Vault USDC ATA — owned by `LobbyEscrow`

## Instructions

| Ix           | Player signs         | Platform signs                                    |
| ------------ | -------------------- | ------------------------------------------------- |
| `initialize` | no                   | yes (becomes platform; creates platform USDC ATA) |
| `join`       | yes (USDC authority) | yes (fee payer + rent)                            |
| `leave`      | no                   | yes                                               |
| `kick`       | no                   | yes                                               |
| `claim`      | no                   | yes                                               |

`claim` takes `amount` + `dev_fee` percent (0–5). The program computes
`platform = amount * 2 / 100` and pays `Config.platform`. The caller cannot
zero out the platform cut. After the first claim, join / leave / kick freeze
(`claims_started`), matching Stacks.

`path_hash` is SHA-256 of the lobby path string.

## Deploy

Wallet: `SOLANA_WARS_KEY` → `.keys/wars-wallet.json` (gitignored).
Program id is `declare_id!` / `Anchor.toml`.

```sh
cd backend/sw-vault/solana
solana config set --url devnet --keypair .keys/wars-wallet.json
# needs ~2 SOL on the cluster (283KB program)
anchor deploy
```

Then run `initialize` once so the deployer is stored as platform and the
platform USDC ATA exists. For devnet the mint is our own 6-decimal
devnet USDC (`2ztYALhLWs2Lg1bGRBje82RgiLhuH4ZbCimRWVeyxUaB`), not Circle.

```sh
cd frontend && node scripts/initialize-solana-vault.mjs
```
