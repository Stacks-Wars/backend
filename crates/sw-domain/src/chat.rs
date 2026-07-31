use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{LobbyId, UserId};

/// Longest accepted chat body, in characters.
pub const CHAT_MAX_LEN: usize = 400;

/// A single lobby chat line (Redis-backed, capped history).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LobbyChatMessage {
    pub id: Uuid,
    pub lobby_id: LobbyId,
    pub user_id: UserId,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub body: String,
    /// Unix seconds.
    pub sent_at: i64,
}

impl LobbyChatMessage {
    pub fn new(
        lobby_id: LobbyId,
        user_id: UserId,
        username: Option<String>,
        display_name: Option<String>,
        body: String,
    ) -> Self {
        Self {
            id: Uuid::now_v7(),
            lobby_id,
            user_id,
            username,
            display_name,
            body,
            sent_at: chrono::Utc::now().timestamp(),
        }
    }
}

/// Trim and length-check a chat body. `None` when the message is unusable.
pub fn sanitize_chat_body(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let body: String = trimmed.chars().take(CHAT_MAX_LEN).collect();
    Some(body)
}
