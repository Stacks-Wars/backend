//! Persisted `lobby.finished` payload so revisiting a finished room can show
//! MatchResult without relying on the one-shot WebSocket event.

use redis::aio::ConnectionManager;
use redis::AsyncCommands;
use serde_json::Value;
use sw_domain::LobbyId;

use crate::error::{AppError, AppResult};

/// Keep finished payloads long enough for rematches / share links.
const TTL_SECS: i64 = 7 * 24 * 60 * 60;

fn key(lobby_id: LobbyId) -> String {
    format!("lobby:{}:finished", lobby_id.as_uuid())
}

pub struct LobbyFinishedRepo {
    redis: ConnectionManager,
}

impl LobbyFinishedRepo {
    pub fn new(redis: ConnectionManager) -> Self {
        Self { redis }
    }

    pub async fn set(&self, lobby_id: LobbyId, payload: &Value) -> AppResult<()> {
        let mut redis = self.redis.clone();
        let raw =
            serde_json::to_string(payload).map_err(|e| AppError::Internal(e.into()))?;
        redis
            .set_ex::<_, _, ()>(key(lobby_id), raw, TTL_SECS as u64)
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        Ok(())
    }

    pub async fn get(&self, lobby_id: LobbyId) -> AppResult<Option<Value>> {
        let mut redis = self.redis.clone();
        let raw: Option<String> = redis
            .get(key(lobby_id))
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        match raw {
            Some(raw) => {
                let value = serde_json::from_str(&raw)
                    .map_err(|e| AppError::Internal(e.into()))?;
                Ok(Some(value))
            }
            None => Ok(None),
        }
    }

    /// Flip `needsOnChainClaim` off after a successful claim confirm.
    pub async fn mark_claimed(&self, lobby_id: LobbyId) -> AppResult<()> {
        let Some(mut value) = self.get(lobby_id).await? else {
            return Ok(());
        };
        if let Some(obj) = value.as_object_mut() {
            obj.insert("needsOnChainClaim".into(), Value::Bool(false));
        }
        self.set(lobby_id, &value).await
    }
}
