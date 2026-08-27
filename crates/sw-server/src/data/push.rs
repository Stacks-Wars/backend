//! Web Push subscriptions stored per user.

use sqlx::PgPool;
use sw_domain::UserId;
use uuid::Uuid;

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PushSubscription {
    pub endpoint: String,
    pub user_id: Uuid,
    pub p256dh: String,
    pub auth: String,
    pub user_agent: Option<String>,
}

pub struct PushSubscriptionRepo {
    pool: PgPool,
}

impl PushSubscriptionRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn upsert(
        &self,
        user_id: UserId,
        endpoint: &str,
        p256dh: &str,
        auth: &str,
        user_agent: Option<&str>,
    ) -> AppResult<()> {
        sqlx::query(
            r#"
            INSERT INTO push_subscriptions (endpoint, user_id, p256dh, auth, user_agent)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (endpoint) DO UPDATE SET
                user_id = EXCLUDED.user_id,
                p256dh = EXCLUDED.p256dh,
                auth = EXCLUDED.auth,
                user_agent = EXCLUDED.user_agent,
                updated_at = now()
            "#,
        )
        .bind(endpoint)
        .bind(user_id.as_uuid())
        .bind(p256dh)
        .bind(auth)
        .bind(user_agent)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Internal(e.into()))?;
        Ok(())
    }

    pub async fn delete_endpoint(&self, user_id: UserId, endpoint: &str) -> AppResult<()> {
        sqlx::query(
            r#"
            DELETE FROM push_subscriptions
            WHERE user_id = $1 AND endpoint = $2
            "#,
        )
        .bind(user_id.as_uuid())
        .bind(endpoint)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Internal(e.into()))?;
        Ok(())
    }

    pub async fn delete_all_for_user(&self, user_id: UserId) -> AppResult<()> {
        sqlx::query(r#"DELETE FROM push_subscriptions WHERE user_id = $1"#)
            .bind(user_id.as_uuid())
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        Ok(())
    }

    pub async fn list_for_user(&self, user_id: UserId) -> AppResult<Vec<PushSubscription>> {
        sqlx::query_as::<_, PushSubscription>(
            r#"
            SELECT endpoint, user_id, p256dh, auth, user_agent
            FROM push_subscriptions
            WHERE user_id = $1
            "#,
        )
        .bind(user_id.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::Internal(e.into()))
    }

    /// Public-lobby fanout: subscribers with lobby alerts on.
    ///
    /// `paid_chain` scopes paid/sponsored lobbies to users currently on that
    /// chain. Free lobbies pass `None` and reach every alert subscriber.
    pub async fn list_lobby_alert_targets(
        &self,
        except_user: UserId,
        paid_chain: Option<sw_domain::ChainId>,
    ) -> AppResult<Vec<PushSubscription>> {
        let chain = paid_chain.map(|c| c.as_str());
        sqlx::query_as::<_, PushSubscription>(
            r#"
            SELECT s.endpoint, s.user_id, s.p256dh, s.auth, s.user_agent
            FROM push_subscriptions s
            INNER JOIN users u ON u.id = s.user_id
            WHERE u.lobby_alerts_enabled = true
              AND u.deleted_at IS NULL
              AND s.user_id <> $1
              AND ($2::TEXT IS NULL OR u.current_chain::text = $2)
            "#,
        )
        .bind(except_user.as_uuid())
        .bind(chain)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::Internal(e.into()))
    }
}
