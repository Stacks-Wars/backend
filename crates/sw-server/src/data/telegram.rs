//! Redis store for Telegram message ids + notification idempotency flags.

use redis::AsyncCommands;
use redis::aio::ConnectionManager;
use serde::{Deserialize, Serialize};
use sw_domain::LobbyId;

use crate::error::{AppError, AppResult};

const TTL_SECS: i64 = 7 * 24 * 60 * 60;

fn key(lobby_id: LobbyId) -> String {
    format!("lobby:{}:telegram", lobby_id.as_uuid())
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TelegramLobbyMsg {
    pub message_id: i64,
    #[serde(default)]
    pub finished_notified: bool,
}

pub struct TelegramMsgRepo {
    redis: ConnectionManager,
}

impl TelegramMsgRepo {
    pub fn new(redis: ConnectionManager) -> Self {
        Self { redis }
    }

    pub async fn get(&self, lobby_id: LobbyId) -> AppResult<Option<TelegramLobbyMsg>> {
        let mut redis = self.redis.clone();
        let raw: Option<String> = redis
            .get(key(lobby_id))
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        match raw {
            Some(raw) => {
                let value = serde_json::from_str(&raw).map_err(|e| AppError::Internal(e.into()))?;
                Ok(Some(value))
            }
            None => Ok(None),
        }
    }

    /// Atomically claim the create-notification slot. Returns `true` if this
    /// caller should send the Telegram announcement.
    pub async fn try_claim_create(&self, lobby_id: LobbyId) -> AppResult<bool> {
        let mut redis = self.redis.clone();
        let placeholder = TelegramLobbyMsg {
            message_id: 0,
            finished_notified: false,
        };
        let raw = serde_json::to_string(&placeholder).map_err(|e| AppError::Internal(e.into()))?;
        let set: bool = redis
            .set_nx(key(lobby_id), raw)
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        if set {
            let _: Result<(), _> = redis.expire(key(lobby_id), TTL_SECS).await;
        }
        Ok(set)
    }

    pub async fn set_message_id(&self, lobby_id: LobbyId, message_id: i64) -> AppResult<()> {
        let mut redis = self.redis.clone();
        let value = TelegramLobbyMsg {
            message_id,
            finished_notified: false,
        };
        let raw = serde_json::to_string(&value).map_err(|e| AppError::Internal(e.into()))?;
        redis
            .set_ex::<_, _, ()>(key(lobby_id), raw, TTL_SECS as u64)
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        Ok(())
    }

    pub async fn set_finished(&self, lobby_id: LobbyId, message_id: i64) -> AppResult<()> {
        let mut redis = self.redis.clone();
        let value = TelegramLobbyMsg {
            message_id,
            finished_notified: true,
        };
        let raw = serde_json::to_string(&value).map_err(|e| AppError::Internal(e.into()))?;
        redis
            .set_ex::<_, _, ()>(key(lobby_id), raw, TTL_SECS as u64)
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        Ok(())
    }

    pub async fn take(&self, lobby_id: LobbyId) -> AppResult<Option<TelegramLobbyMsg>> {
        let mut redis = self.redis.clone();
        let k = key(lobby_id);
        let raw: Option<String> = redis
            .get(&k)
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        let _: Result<(), _> = redis.del(&k).await;
        match raw {
            Some(raw) => {
                let value = serde_json::from_str(&raw).map_err(|e| AppError::Internal(e.into()))?;
                Ok(Some(value))
            }
            None => Ok(None),
        }
    }

    pub async fn clear(&self, lobby_id: LobbyId) -> AppResult<()> {
        let mut redis = self.redis.clone();
        let _: Result<(), _> = redis.del(key(lobby_id)).await;
        Ok(())
    }
}
