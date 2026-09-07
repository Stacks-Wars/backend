# sw-vault

On-chain escrow for paid lobbies.

| Chain | Path | Runtime |
| --- | --- | --- |
| Stacks | [`stacks/`](./stacks) | Clarinet / Clarity (`sw-vault-v1`) |
| Solana | [`solana/programs/src/lib.rs`](./solana/programs/src/lib.rs) | Anchor / USDC token accounts |
| Arbitrum | [`arbitrum/`](./arbitrum) | Foundry / USDCx + `SwVault` on Sepolia |

The Next.js app sponsors fees: Stacks via `sponsorTransaction`, Solana via a platform fee-payer on `@solana/kit`, Arbitrum via the platform EOA paying gas while players sign ERC-2612 permit. Players do not need the native gas token.

Do not 1:1-port the Clarity maps onto Solana. Solana uses a lobby PDA + USDC ATAs, with the platform key as a remaining signer on leave / kick / claim. Arbitrum keeps the same lobby semantics in Solidity (`join` / `leaveSeat` / `kick` / `claim`). `leave` is not the Solidity identifier — it will become a keyword.
