use serde::{Deserialize, Serialize};
use sw_domain::UserId;

/// Direction-agnostic envelope exchanged between clients, host, and engines.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameMessage {
    pub kind: String,
    pub payload: serde_json::Value,
}

/// Player lifecycle / input events delivered into an engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PlayerEvent {
    Joined { user_id: UserId },
    Left { user_id: UserId },
    Ready { user_id: UserId, ready: bool },
}

/// Outcome reported when an engine finishes a match.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchResult {
    pub winners: Vec<UserId>,
    pub rankings: Vec<UserId>,
    /// Opaque per-game stats for later persistence / wars points.
    pub stats: serde_json::Value,
}
