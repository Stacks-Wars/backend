use chrono::Utc;
use redis::AsyncCommands;
use redis::aio::ConnectionManager;
use serde::{Deserialize, Serialize};
use sw_domain::{JoinRequestState, LobbyId, UserId};

use crate::error::{AppError, AppResult};

/// Join requests expire with the waiting lobby.
const TTL_SECS: i64 = 15 * 60;

fn join_requests_key(lobby_id: LobbyId) -> String {
    format!("lobby:{}:join_requests", lobby_id.as_uuid())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JoinRequest {
    pub user_id: UserId,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub state: JoinRequestState,
    pub created_at: i64,
}

pub struct JoinRequestRepo {
    redis: ConnectionManager,
}

impl JoinRequestRepo {
    pub fn new(redis: ConnectionManager) -> Self {
        Self { redis }
    }

    async fn refresh_ttl(&self, key: &str) -> AppResult<()> {
        let mut redis = self.redis.clone();
        redis
            .expire::<_, ()>(key, TTL_SECS)
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        Ok(())
    }

    pub async fn upsert(&self, lobby_id: LobbyId, request: &JoinRequest) -> AppResult<()> {
        let mut redis = self.redis.clone();
        let key = join_requests_key(lobby_id);
        let payload = serde_json::to_string(request).map_err(|e| AppError::Internal(e.into()))?;
        redis
            .hset::<_, _, _, ()>(&key, request.user_id.as_uuid().to_string(), payload)
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        self.refresh_ttl(&key).await
    }

    pub async fn get(&self, lobby_id: LobbyId, user_id: UserId) -> AppResult<Option<JoinRequest>> {
        let mut redis = self.redis.clone();
        let key = join_requests_key(lobby_id);
        let raw: Option<String> = redis
            .hget(&key, user_id.as_uuid().to_string())
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        match raw {
            Some(raw) => {
                let request =
                    serde_json::from_str(&raw).map_err(|e| AppError::Internal(e.into()))?;
                Ok(Some(request))
            }
            None => Ok(None),
        }
    }

    pub async fn list(&self, lobby_id: LobbyId) -> AppResult<Vec<JoinRequest>> {
        let mut redis = self.redis.clone();
        let key = join_requests_key(lobby_id);
        let map: std::collections::HashMap<String, String> = redis
            .hgetall(&key)
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        let mut out: Vec<JoinRequest> = Vec::with_capacity(map.len());
        for raw in map.into_values() {
            if let Ok(request) = serde_json::from_str(&raw) {
                out.push(request);
            }
        }
        out.sort_by_key(|jr| jr.created_at);
        Ok(out)
    }

    pub async fn set_state(
        &self,
        lobby_id: LobbyId,
        user_id: UserId,
        state: JoinRequestState,
    ) -> AppResult<Option<JoinRequest>> {
        let mut redis = self.redis.clone();
        let key = join_requests_key(lobby_id);
        let field = user_id.as_uuid().to_string();
        let raw: Option<String> = redis
            .hget(&key, &field)
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        let Some(raw) = raw else {
            return Ok(None);
        };
        let mut request: JoinRequest =
            serde_json::from_str(&raw).map_err(|e| AppError::Internal(e.into()))?;
        request.state = state;
        let payload = serde_json::to_string(&request).map_err(|e| AppError::Internal(e.into()))?;
        redis
            .hset::<_, _, _, ()>(&key, field, payload)
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        self.refresh_ttl(&key).await?;
        Ok(Some(request))
    }

    pub async fn delete(&self, lobby_id: LobbyId, user_id: UserId) -> AppResult<()> {
        let mut redis = self.redis.clone();
        let key = join_requests_key(lobby_id);
        redis
            .hdel::<_, _, ()>(&key, user_id.as_uuid().to_string())
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        Ok(())
    }

    pub async fn clear_lobby(&self, lobby_id: LobbyId) -> AppResult<()> {
        let mut redis = self.redis.clone();
        redis
            .del::<_, ()>(join_requests_key(lobby_id))
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        Ok(())
    }
}

impl JoinRequest {
    pub fn pending(
        user_id: UserId,
        username: Option<String>,
        display_name: Option<String>,
    ) -> Self {
        Self {
            user_id,
            username,
            display_name,
            state: JoinRequestState::Pending,
            created_at: Utc::now().timestamp(),
        }
    }
}
