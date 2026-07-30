//! On-chain wallet DTOs (no ledger). Balance SoT is Hiro FT balance.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::UserId;

/// Custodial USDCx balance mirrored from chain (optionally Redis-cached).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WalletBalance {
    pub user_id: UserId,
    pub stx_address: String,
    pub available_micro: i64,
    pub updated_at: DateTime<Utc>,
    /// True when served from Redis cache.
    #[serde(default)]
    pub cached: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChainActivityKind {
    Deposit,
    Withdraw,
    VaultJoin,
    VaultLeave,
    VaultKick,
    VaultClaim,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChainActivityItem {
    pub txid: String,
    pub kind: ChainActivityKind,
    pub amount_micro: i64,
    pub from_address: Option<String>,
    pub to_address: Option<String>,
    pub lobby_path: Option<String>,
    pub status: String,
    pub block_time: Option<i64>,
}
