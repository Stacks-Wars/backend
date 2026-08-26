//! STACKS_WARS_KEY oracle helpers — fee split preview + single claim intent.
//!
//! Clarity message signing and sponsored fee payment live in the Next.js app
//! (`frontend/lib/vault/`) using the same `STACKS_WARS_KEY` mnemonic. This
//! module is the shared settle math used by `ServerGameHost`.

use sw_domain::UserId;

/// Split pot into platform (2%), game fee (0–5%), and winner remainder.
/// Mirrors Clarity `calculate-split` / `PLATFORM-FEE`.
pub fn split_pot(pot_micro: i64, game_fee_pct: u8) -> (i64, i64, i64) {
    if pot_micro <= 0 {
        return (0, 0, 0);
    }
    let game_fee_pct = game_fee_pct.min(5);
    let platform = (pot_micro * 2) / 100;
    let dest = (pot_micro * game_fee_pct as i64) / 100;
    let winner = pot_micro - platform - dest;
    (platform, dest, winner)
}

#[derive(Debug, Clone)]
pub struct VaultClaimIntent {
    /// Winner custodial user (tx-sender for on-chain claim).
    pub user_id: UserId,
    /// Winner custodial principal.
    pub principal: String,
    /// Full pot amount signed into the claim (contract splits it).
    pub amount_micro: i64,
    pub nonce: u64,
    /// Dest fee recipient principal (ignored on-chain when `dest_fee` is 0).
    pub dest_wallet: String,
    /// Dest fee percent (0–5); `0` → winner + platform only.
    pub dest_fee: u8,
    /// Plugin dest user. Present when that user exists on this environment.
    pub dest_id: Option<UserId>,
    /// Dest user exists but has no wallet on this lobby's chain yet.
    pub dest_needs_wallet: bool,
}

/// Build a single claim intent for the winner. Contract pays platform + optional
/// game fee from the signed pot amount in the same transaction.
///
/// When `game_fee_pct` is 0, `dest_wallet` is still required by the ABI but is
/// unused on-chain — callers may pass the platform principal as a placeholder.
pub fn build_claim_intent(
    pot_micro: i64,
    game_fee_pct: u8,
    winner: UserId,
    winner_principal: String,
    dest_wallet: String,
    dest_id: Option<UserId>,
    dest_needs_wallet: bool,
) -> Option<VaultClaimIntent> {
    if pot_micro <= 0 || winner_principal.is_empty() || dest_wallet.is_empty() {
        return None;
    }
    let dest_fee = game_fee_pct.min(5);
    Some(VaultClaimIntent {
        user_id: winner,
        principal: winner_principal,
        amount_micro: pot_micro,
        nonce: 1,
        dest_wallet,
        dest_fee,
        dest_id,
        dest_needs_wallet,
    })
}
