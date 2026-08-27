# sw-vault

On-chain escrow for paid lobbies. Stacks is live. Solana is next.

| Chain | Path | Runtime |
| --- | --- | --- |
| Stacks | [`stacks/`](./stacks) | Clarinet / Clarity (`sw-vault-v1`) |
| Solana | [`solana/programs/src/lib.rs`](./solana/programs/src/lib.rs) | Anchor / USDC token accounts |

The Next.js app sponsors fees on both chains: Stacks via `sponsorTransaction`, Solana via a platform fee-payer on `@solana/kit` (`lib/solana/sponsor.ts`). Players sign as the token authority and do not need the native gas token.

Do not 1:1-port the Clarity maps onto Solana. Solana uses a lobby PDA + USDC ATAs, with the platform key as a remaining signer on leave / kick / claim.
