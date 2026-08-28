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

/// Winner-take-pot only when the engine named a winner. Empty `winners` is a
/// draw (or unset) — never fall back to `rankings[0]`.
pub fn winner_for_claim(result: &sw_plugin::MatchResult) -> Option<UserId> {
    result.winners.first().copied()
}

pub fn is_draw_result(result: &sw_plugin::MatchResult) -> bool {
    result.winners.is_empty()
}

/// Entry actually paid by this seat. Sponsored guests pay nothing.
pub fn seat_paid_micro(
    entry_amount_micro: i64,
    is_sponsored: bool,
    creator_id: UserId,
    user_id: UserId,
) -> i64 {
    if entry_amount_micro <= 0 {
        0
    } else if is_sponsored && creator_id != user_id {
        0
    } else {
        entry_amount_micro
    }
}

/// Per-seat refund descriptors for a paid draw. Unpaid seats (free lobby,
/// sponsored guests, missing wallet) are omitted — no winner claim.
pub fn draw_refund_claims(
    seats: impl IntoIterator<Item = (UserId, Option<String>)>,
    entry_amount_micro: i64,
    is_sponsored: bool,
    creator_id: UserId,
) -> Vec<serde_json::Value> {
    seats
        .into_iter()
        .enumerate()
        .filter_map(|(index, (user_id, principal))| {
            let principal = principal.filter(|p| !p.is_empty())?;
            let paid = seat_paid_micro(entry_amount_micro, is_sponsored, creator_id, user_id);
            if paid <= 0 {
                return None;
            }
            Some(serde_json::json!({
                "userId": user_id.as_uuid().to_string(),
                "principal": principal,
                "amountMicro": paid,
                "nonce": index as u64 + 1,
                "devWallet": "",
                "devFee": 0,
                "role": "refund",
            }))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sw_plugin::MatchResult;

    fn uid(n: u8) -> UserId {
        UserId::from(uuid::Uuid::from_bytes([n; 16]))
    }

    #[test]
    fn draw_has_no_claim_winner() {
        let draw = MatchResult {
            winners: vec![],
            rankings: vec![uid(1), uid(2)],
            stats: serde_json::json!({ "outcome": "draw" }),
        };
        assert!(is_draw_result(&draw));
        assert_eq!(winner_for_claim(&draw), None);
    }

    #[test]
    fn named_winner_is_claimed_not_first_ranking() {
        let result = MatchResult {
            winners: vec![uid(2)],
            rankings: vec![uid(1), uid(2)],
            stats: serde_json::json!({}),
        };
        assert!(!is_draw_result(&result));
        assert_eq!(winner_for_claim(&result), Some(uid(2)));
    }

    #[test]
    fn draw_settle_builds_refunds_not_a_winner_claim() {
        let a = uid(1);
        let b = uid(2);
        let draw = MatchResult {
            winners: vec![],
            rankings: vec![a, b],
            stats: serde_json::json!({ "outcome": "draw" }),
        };
        assert!(is_draw_result(&draw));
        assert_eq!(winner_for_claim(&draw), None);

        let claims = draw_refund_claims(
            vec![
                (a, Some("SP1".into())),
                (b, Some("SP2".into())),
            ],
            1_000_000,
            false,
            a,
        );
        assert_eq!(claims.len(), 2);
        assert!(claims.iter().all(|c| c["role"] == "refund"));
        assert!(claims.iter().all(|c| c["amountMicro"] == 1_000_000));
        assert!(claims.iter().all(|c| c["devFee"] == 0));
        assert_eq!(claims[0]["userId"], a.as_uuid().to_string());
        assert_eq!(claims[1]["userId"], b.as_uuid().to_string());
    }

    #[test]
    fn sponsored_draw_refunds_only_the_creator_entry() {
        let creator = uid(1);
        let guest = uid(2);
        let claims = draw_refund_claims(
            vec![
                (creator, Some("SP1".into())),
                (guest, Some("SP2".into())),
            ],
            2_000_000,
            true,
            creator,
        );
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0]["userId"], creator.as_uuid().to_string());
        assert_eq!(claims[0]["amountMicro"], 2_000_000);
        assert_eq!(claims[0]["role"], "refund");
    }
}
