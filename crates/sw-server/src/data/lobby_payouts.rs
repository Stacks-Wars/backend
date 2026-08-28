//! Mid-match vault claim intents. Games that split the pot call
//! `GameHost::issue_payout` as ranks lock; settle reads the same list so
//! `complete_match` does not issue a second winner-take-all claim.

use redis::AsyncCommands;
use redis::aio::ConnectionManager;
use serde_json::Value;
use sw_domain::{LobbyId, UserId};

use crate::error::{AppError, AppResult};

const TTL_SECS: i64 = 7 * 24 * 60 * 60;

fn list_key(lobby_id: LobbyId) -> String {
    format!("lobby:{}:payouts", lobby_id.as_uuid())
}

fn nonce_key(lobby_id: LobbyId) -> String {
    format!("lobby:{}:claim-nonce", lobby_id.as_uuid())
}

pub struct LobbyPayoutRepo {
    redis: ConnectionManager,
}

impl LobbyPayoutRepo {
    pub fn new(redis: ConnectionManager) -> Self {
        Self { redis }
    }

    pub async fn next_nonce(&self, lobby_id: LobbyId) -> AppResult<u64> {
        let mut redis = self.redis.clone();
        let n: i64 = redis
            .incr(nonce_key(lobby_id), 1)
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        let _: () = redis
            .expire(nonce_key(lobby_id), TTL_SECS)
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        Ok(n.max(1) as u64)
    }

    pub async fn already_paid(&self, lobby_id: LobbyId, user_id: UserId) -> AppResult<bool> {
        let claims = self.list(lobby_id).await?;
        let uid = user_id.as_uuid().to_string();
        Ok(claims.iter().any(|claim| {
            claim.get("userId").and_then(|v| v.as_str()) == Some(uid.as_str())
                && claim.get("role").and_then(|v| v.as_str()) != Some("refund")
        }))
    }

    pub async fn push(&self, lobby_id: LobbyId, claim: &Value) -> AppResult<()> {
        let mut redis = self.redis.clone();
        let raw = serde_json::to_string(claim).map_err(|e| AppError::Internal(e.into()))?;
        redis
            .rpush::<_, _, ()>(list_key(lobby_id), raw)
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        let _: () = redis
            .expire(list_key(lobby_id), TTL_SECS)
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        Ok(())
    }

    pub async fn list(&self, lobby_id: LobbyId) -> AppResult<Vec<Value>> {
        let mut redis = self.redis.clone();
        let raw: Vec<String> = redis
            .lrange(list_key(lobby_id), 0, -1)
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        Ok(raw
            .into_iter()
            .filter_map(|s| serde_json::from_str(&s).ok())
            .collect())
    }
}
