//! Wire types shared between the server and game crates.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PlayerStatus {
    NotJoined,
    Joined,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", content = "data", rename_all = "camelCase")]
pub enum ClaimState {
    Claimed { tx_id: String },
    NotClaimed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum JoinRequestState {
    Pending,
    Accepted,
    Rejected,
}

/// Runtime player row games may embed in events (matches Redis `PlayerState` JSON).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerStateWire {
    pub user_id: Uuid,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub lobby_id: Uuid,
    pub status: PlayerStatus,
    pub state: JoinRequestState,
    pub rank: Option<usize>,
    pub prize: Option<f64>,
    pub wars_point: Option<f64>,
    pub claim_state: Option<ClaimState>,
    pub last_ping: Option<u64>,
    pub joined_at: i64,
    pub updated_at: i64,
    pub is_creator: bool,
}
