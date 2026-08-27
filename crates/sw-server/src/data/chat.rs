use redis::AsyncCommands;
use redis::aio::ConnectionManager;
use sw_domain::{LobbyChatMessage, LobbyId};

use crate::error::{AppError, AppResult};

/// Lines kept per lobby; older lines are trimmed away.
const HISTORY_LIMIT: isize = 80;
/// Chat outlives a match but not the server's memory of the lobby.
const HISTORY_TTL_SECS: i64 = 60 * 60 * 12;

fn chat_key(lobby_id: LobbyId) -> String {
    format!("lobby:{}:chat", lobby_id.as_uuid())
}

pub struct LobbyChatRepo {
    redis: ConnectionManager,
}

impl LobbyChatRepo {
    pub fn new(redis: ConnectionManager) -> Self {
        Self { redis }
    }

    pub async fn append(&self, message: &LobbyChatMessage) -> AppResult<()> {
        let mut redis = self.redis.clone();
        let key = chat_key(message.lobby_id);
        let payload = serde_json::to_string(message).map_err(|e| AppError::Internal(e.into()))?;
        redis
            .rpush::<_, _, ()>(&key, payload)
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        redis
            .ltrim::<_, ()>(&key, -HISTORY_LIMIT, -1)
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        redis
            .expire::<_, ()>(&key, HISTORY_TTL_SECS)
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        Ok(())
    }

    /// Oldest to newest.
    pub async fn history(&self, lobby_id: LobbyId) -> AppResult<Vec<LobbyChatMessage>> {
        let mut redis = self.redis.clone();
        let raw: Vec<String> = redis
            .lrange(chat_key(lobby_id), 0, -1)
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        Ok(raw
            .iter()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect())
    }

    pub async fn clear(&self, lobby_id: LobbyId) -> AppResult<()> {
        let mut redis = self.redis.clone();
        redis
            .del::<_, ()>(chat_key(lobby_id))
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        Ok(())
    }
}
