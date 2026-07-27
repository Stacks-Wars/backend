use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{GameId, LobbyId, UserId};

/// Lobby lifecycle. Transitions are not enforced in the shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LobbyStatus {
    /// Accepting players.
    Open,
    /// Roster frozen; waiting to start.
    Locked,
    /// Pre-match countdown.
    Countdown,
    /// Engine is running.
    InProgress,
    /// Match finished; settling prizes / points.
    Settling,
    /// Terminal success.
    Completed,
    /// Terminal abort.
    Cancelled,
}

impl LobbyStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled)
    }
}

/// How entry / prizes are funded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StakeMode {
    /// No stake.
    Free,
    /// Players stake STX or an FT.
    Staked,
    /// Organizer / sponsor covers the prize pool.
    Sponsored,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LobbyPlayer {
    pub user_id: UserId,
    pub joined_at: DateTime<Utc>,
    pub ready: bool,
}

/// Matchmaking room for a single game instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lobby {
    pub id: LobbyId,
    pub game_id: GameId,
    pub host_user_id: UserId,
    pub status: LobbyStatus,
    pub stake_mode: StakeMode,
    /// Opaque game-specific settings (validated by the game crate later).
    pub settings: serde_json::Value,
    pub players: Vec<LobbyPlayer>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
