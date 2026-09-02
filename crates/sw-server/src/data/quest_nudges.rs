//! Daily quest reminder fanout: claim a send slot, then list push subscriptions.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::data::push::PushSubscription;
use crate::error::{AppError, AppResult};

pub struct QuestNudgeRepo {
    pool: PgPool,
}

impl QuestNudgeRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Eligible subscriptions without claiming a send slot. Used when Web Push
    /// is unavailable so a later cron can still send the OS banner.
    pub async fn list_eligible(
        &self,
        period_id: &str,
        day_start: DateTime<Utc>,
    ) -> AppResult<Vec<PushSubscription>> {
        sqlx::query_as::<_, PushSubscription>(
            r#"
            WITH eligible AS (
                SELECT DISTINCT u.id AS user_id
                FROM users u
                INNER JOIN push_subscriptions s ON s.user_id = u.id
                WHERE u.deleted_at IS NULL
                  AND NOT EXISTS (
                      SELECT 1
                      FROM match_players mp
                      JOIN matches m ON m.id = mp.match_id
                      WHERE mp.user_id = u.id
                        AND m.player_count >= 2
                        AND m.finished_at >= $1
                  )
                  AND NOT EXISTS (
                      SELECT 1
                      FROM quest_claims c
                      WHERE c.user_id = u.id
                        AND c.period_kind = 'daily'
                        AND c.period_id = $2
                  )
                  AND NOT EXISTS (
                      SELECT 1
                      FROM quest_nudges n
                      WHERE n.user_id = u.id
                        AND n.period_id = $2
                  )
            )
            SELECT s.endpoint, s.user_id, s.p256dh, s.auth, s.user_agent
            FROM push_subscriptions s
            INNER JOIN eligible e ON e.user_id = s.user_id
            "#,
        )
        .bind(day_start)
        .bind(period_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|err| AppError::Internal(err.into()))
    }

    /// Insert nudge rows for eligible users who have not been sent today, and
    /// return their push subscriptions. Only call this when Web Push can send.
    pub async fn claim_and_list(
        &self,
        period_id: &str,
        day_start: DateTime<Utc>,
    ) -> AppResult<Vec<PushSubscription>> {
        sqlx::query_as::<_, PushSubscription>(
            r#"
            WITH eligible AS (
                SELECT DISTINCT u.id AS user_id
                FROM users u
                INNER JOIN push_subscriptions s ON s.user_id = u.id
                WHERE u.deleted_at IS NULL
                  AND NOT EXISTS (
                      SELECT 1
                      FROM match_players mp
                      JOIN matches m ON m.id = mp.match_id
                      WHERE mp.user_id = u.id
                        AND m.player_count >= 2
                        AND m.finished_at >= $1
                  )
                  AND NOT EXISTS (
                      SELECT 1
                      FROM quest_claims c
                      WHERE c.user_id = u.id
                        AND c.period_kind = 'daily'
                        AND c.period_id = $2
                  )
                  AND NOT EXISTS (
                      SELECT 1
                      FROM quest_nudges n
                      WHERE n.user_id = u.id
                        AND n.period_id = $2
                  )
            ),
            inserted AS (
                INSERT INTO quest_nudges (user_id, period_id)
                SELECT user_id, $2
                FROM eligible
                ON CONFLICT (user_id, period_id) DO NOTHING
                RETURNING user_id
            )
            SELECT s.endpoint, s.user_id, s.p256dh, s.auth, s.user_agent
            FROM push_subscriptions s
            INNER JOIN inserted i ON i.user_id = s.user_id
            "#,
        )
        .bind(day_start)
        .bind(period_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|err| AppError::Internal(err.into()))
    }
}

pub fn unique_user_ids(subs: &[PushSubscription]) -> Vec<Uuid> {
    let mut seen = std::collections::HashSet::new();
    subs.iter()
        .filter_map(|sub| seen.insert(sub.user_id).then_some(sub.user_id))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sub(user_id: Uuid) -> PushSubscription {
        PushSubscription {
            endpoint: user_id.to_string(),
            user_id,
            p256dh: "p".into(),
            auth: "a".into(),
            user_agent: None,
        }
    }

    #[test]
    fn unique_user_ids_dedupes() {
        let a = Uuid::nil();
        let b = Uuid::from_u128(1);
        assert_eq!(unique_user_ids(&[sub(a), sub(a), sub(b)]).len(), 2);
    }
}
