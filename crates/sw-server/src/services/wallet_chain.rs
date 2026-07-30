//! On-chain custodial balance + activity (Hiro), Redis-cached for UI reads.

use chrono::Utc;
use redis::aio::ConnectionManager;
use redis::AsyncCommands;
use sw_domain::{ChainActivityItem, UserId, WalletBalance};

use crate::config::BALANCE_CACHE_SECS;
use crate::data::users::PgUserRepo;
use crate::error::{AppError, AppResult};
use crate::services::hiro::HiroClient;
use sqlx::PgPool;

const BALANCE_KEY_PREFIX: &str = "sw:balance:";
const WITHDRAW_LOCK_PREFIX: &str = "sw:withdraw-lock:";

pub struct WalletChainService {
    pool: PgPool,
    redis: ConnectionManager,
    hiro: HiroClient,
}

impl WalletChainService {
    pub fn new(pool: PgPool, redis: ConnectionManager, hiro: HiroClient) -> Self {
        Self { pool, redis, hiro }
    }

    pub fn hiro(&self) -> &HiroClient {
        &self.hiro
    }

    fn balance_key(user_id: UserId) -> String {
        format!("{BALANCE_KEY_PREFIX}{}", user_id.as_uuid())
    }

    fn withdraw_lock_key(user_id: UserId) -> String {
        format!("{WITHDRAW_LOCK_PREFIX}{}", user_id.as_uuid())
    }

    /// UI read — Redis cache (TTL [`BALANCE_CACHE_SECS`]), miss → Hiro.
    pub async fn get_balance(&self, user_id: UserId) -> AppResult<WalletBalance> {
        let mut redis = self.redis.clone();
        let key = Self::balance_key(user_id);
        if let Ok(Some(raw)) = redis.get::<_, Option<String>>(&key).await {
            if let Ok(mut bal) = serde_json::from_str::<WalletBalance>(&raw) {
                bal.cached = true;
                return Ok(bal);
            }
        }
        self.refresh_balance(user_id).await
    }

    /// Validation / post-tx — always Hiro, then rewrite Redis.
    pub async fn refresh_balance(&self, user_id: UserId) -> AppResult<WalletBalance> {
        let wallet = PgUserRepo::new(self.pool.clone())
            .get_custodial_wallet(user_id)
            .await?
            .ok_or(AppError::NotFound("custodial wallet not found"))?;
        let available_micro = self.hiro.get_ft_balance(&wallet.stx_address).await?;
        let bal = WalletBalance {
            user_id,
            stx_address: wallet.stx_address,
            available_micro,
            updated_at: Utc::now(),
            cached: false,
        };
        let mut redis = self.redis.clone();
        let key = Self::balance_key(user_id);
        let payload = serde_json::to_string(&bal).unwrap_or_default();
        let _: Result<(), _> = redis
            .set_ex(key, payload, BALANCE_CACHE_SECS.max(1))
            .await;
        Ok(bal)
    }

    pub async fn bust_balance_cache(&self, user_id: UserId) -> AppResult<()> {
        let mut redis = self.redis.clone();
        let _: Result<(), _> = redis.del(Self::balance_key(user_id)).await;
        Ok(())
    }

    pub async fn activity(
        &self,
        user_id: UserId,
        limit: u32,
    ) -> AppResult<Vec<ChainActivityItem>> {
        let wallet = PgUserRepo::new(self.pool.clone())
            .get_custodial_wallet(user_id)
            .await?
            .ok_or(AppError::NotFound("custodial wallet not found"))?;
        self.hiro
            .list_activity(&wallet.stx_address, limit.clamp(1, 100))
            .await
    }

    /// Acquire a short Redis lock for an in-flight withdrawal (TTL seconds).
    pub async fn acquire_withdraw_lock(&self, user_id: UserId, ttl_secs: u64) -> AppResult<()> {
        let mut redis = self.redis.clone();
        let key = Self::withdraw_lock_key(user_id);
        let ok: bool = redis
            .set_nx(&key, "1")
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        if !ok {
            return Err(AppError::Conflict(
                "a withdrawal is already in progress".into(),
            ));
        }
        let _: Result<(), _> = redis.expire(&key, ttl_secs as i64).await;
        Ok(())
    }

    pub async fn release_withdraw_lock(&self, user_id: UserId) -> AppResult<()> {
        let mut redis = self.redis.clone();
        let _: Result<(), _> = redis.del(Self::withdraw_lock_key(user_id)).await;
        Ok(())
    }
}
