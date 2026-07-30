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
    /// Prize in micro-USDCx.
    pub prize_micro: Option<i64>,
    pub wars_point: Option<i64>,
    pub last_ping: Option<u64>,
    pub joined_at: i64,
    pub updated_at: i64,
    pub is_creator: bool,
    #[serde(default)]
    pub ready: bool,
}
