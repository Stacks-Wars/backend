//! Pending vault txs that confirmed on-chain before the matching lobby API
//! call finished. Lets a retry reuse the txid instead of broadcasting again.

use redis::aio::ConnectionManager;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use sw_domain::UserId;
use uuid::Uuid;

use crate::error::{AppError, AppResult};

const TTL_SECS: i64 = 24 * 60 * 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultDraft {
    pub kind: String,
    pub user_id: Uuid,
    pub lobby_path: String,
    pub lobby_id: Option<Uuid>,
    pub txid: String,
    pub entry_amount_micro: i64,
    pub transfer_micro: Option<i64>,
    pub sponsored: Option<bool>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub game_id: Option<String>,
    pub is_private: Option<bool>,
    pub is_sponsored: Option<bool>,
    pub amount_micro: Option<i64>,
    pub nonce: Option<u64>,
    pub paid_micro: Option<i64>,
    pub dev_wallet: Option<String>,
    pub dev_fee: Option<i64>,
    pub created_at: i64,
}

pub struct VaultDraftRepo {
    redis: ConnectionManager,
}

impl VaultDraftRepo {
    pub fn new(redis: ConnectionManager) -> Self {
        Self { redis }
    }

    fn key(user_id: UserId, kind: &str, lobby_path: &str) -> String {
        format!(
            "sw:vault-draft:{}:{}:{}",
            user_id.as_uuid(),
            kind,
            lobby_path
        )
    }

    fn index_key(user_id: UserId) -> String {
        format!("sw:vault-drafts:{}", user_id.as_uuid())
    }

    pub async fn save(&self, draft: &VaultDraft) -> AppResult<()> {
        let mut redis = self.redis.clone();
        let key = Self::key(
            UserId::from(draft.user_id),
            &draft.kind,
            &draft.lobby_path,
        );
        let payload =
            serde_json::to_string(draft).map_err(|e| AppError::Internal(e.into()))?;
        redis
            .set_ex::<_, _, ()>(&key, payload, TTL_SECS as u64)
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        let index = Self::index_key(UserId::from(draft.user_id));
        redis
            .sadd::<_, _, ()>(&index, &key)
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        redis
            .expire::<_, ()>(&index, TTL_SECS)
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        Ok(())
    }

    pub async fn get(
        &self,
        user_id: UserId,
        kind: &str,
        lobby_path: &str,
    ) -> AppResult<Option<VaultDraft>> {
        let mut redis = self.redis.clone();
        let key = Self::key(user_id, kind, lobby_path);
        let raw: Option<String> = redis
            .get(&key)
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        raw.map(|s| serde_json::from_str(&s).map_err(|e| AppError::Internal(e.into())))
            .transpose()
    }

    /// Newest draft of `kind` for the user, if any.
    pub async fn latest_of_kind(
        &self,
        user_id: UserId,
        kind: &str,
    ) -> AppResult<Option<VaultDraft>> {
        let mut redis = self.redis.clone();
        let index = Self::index_key(user_id);
        let keys: Vec<String> = redis
            .smembers(&index)
            .await
            .map_err(|e| AppError::Internal(e.into()))?;

        let mut newest: Option<VaultDraft> = None;
        for key in keys {
            if !key.contains(&format!(":{kind}:")) {
                continue;
            }
            let raw: Option<String> = redis
                .get(&key)
                .await
                .map_err(|e| AppError::Internal(e.into()))?;
            let Some(raw) = raw else {
                let _: Result<(), _> = redis.srem(&index, &key).await;
                continue;
            };
            let draft: VaultDraft =
                serde_json::from_str(&raw).map_err(|e| AppError::Internal(e.into()))?;
            if newest
                .as_ref()
                .map(|n| draft.created_at > n.created_at)
                .unwrap_or(true)
            {
                newest = Some(draft);
            }
        }
        Ok(newest)
    }

    pub async fn list(&self, user_id: UserId) -> AppResult<Vec<VaultDraft>> {
        let mut redis = self.redis.clone();
        let index = Self::index_key(user_id);
        let keys: Vec<String> = redis
            .smembers(&index)
            .await
            .map_err(|e| AppError::Internal(e.into()))?;

        let mut drafts = Vec::new();
        for key in keys {
            let raw: Option<String> = redis
                .get(&key)
                .await
                .map_err(|e| AppError::Internal(e.into()))?;
            let Some(raw) = raw else {
                let _: Result<(), _> = redis.srem(&index, &key).await;
                continue;
            };
            if let Ok(draft) = serde_json::from_str::<VaultDraft>(&raw) {
                drafts.push(draft);
            }
        }
        drafts.sort_by_key(|d| std::cmp::Reverse(d.created_at));
        Ok(drafts)
    }

    pub async fn delete(
        &self,
        user_id: UserId,
        kind: &str,
        lobby_path: &str,
    ) -> AppResult<()> {
        let mut redis = self.redis.clone();
        let key = Self::key(user_id, kind, lobby_path);
        let index = Self::index_key(user_id);
        let _: Result<(), _> = redis.del(&key).await;
        let _: Result<(), _> = redis.srem(&index, &key).await;
        Ok(())
    }
}
