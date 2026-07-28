use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{GameId, LobbyId, UserId};

/// Lobby lifecycle (Postgres + Redis).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LobbyStatus {
    Waiting,
    Starting,
    InProgress,
    Finished,
}

impl LobbyStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Finished)
    }

    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::Waiting => "waiting",
            Self::Starting => "starting",
            Self::InProgress => "in_progress",
            Self::Finished => "finished",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlayerStatus {
    NotJoined,
    Joined,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JoinRequestState {
    Pending,
    Accepted,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", content = "data", rename_all = "camelCase")]
pub enum ClaimState {
    Claimed { tx_id: String },
    NotClaimed,
}

/// Optional on-chain / custodial address (USDCx contract later).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WalletAddress(pub String);

impl WalletAddress {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for WalletAddress {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for WalletAddress {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

/// Durable lobby row (Postgres).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Lobby {
    pub id: LobbyId,
    pub path: String,
    pub name: String,
    pub description: Option<String>,
    pub game_id: GameId,
    pub creator_id: UserId,
    pub entry_amount: Option<f64>,
    pub current_amount: Option<f64>,
    pub contract_address: Option<WalletAddress>,
    pub is_private: bool,
    pub is_sponsored: bool,
    pub status: LobbyStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub participants: Vec<UserId>,
}

/// Hot lobby runtime (Redis).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LobbyState {
    pub lobby_id: LobbyId,
    pub status: LobbyStatus,
    pub participant_count: usize,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub creator_last_ping: Option<u64>,
}

impl LobbyState {
    pub fn new(lobby_id: LobbyId, participant_count: usize) -> Self {
        Self {
            lobby_id,
            status: LobbyStatus::Waiting,
            participant_count,
            started_at: None,
            finished_at: None,
            creator_last_ping: None,
        }
    }
}

/// Hot per-player runtime (Redis).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerState {
    pub user_id: UserId,
    pub username: Option<String>,
    pub display_name: Option<String>,
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

impl PlayerState {
    pub fn creator(
        user_id: UserId,
        username: Option<String>,
        display_name: Option<String>,
    ) -> Self {
        let now = Utc::now().timestamp();
        Self {
            user_id,
            username,
            display_name,
            status: PlayerStatus::Joined,
            state: JoinRequestState::Accepted,
            rank: None,
            prize: None,
            wars_point: None,
            claim_state: None,
            last_ping: Some(now as u64),
            joined_at: now,
            updated_at: now,
            is_creator: true,
        }
    }
}
