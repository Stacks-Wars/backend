//! Quest claim ledger and the per-user match reads GET/claim evaluate from.

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};
use sw_domain::{LeaderboardEntry, SeasonId, UserId};
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::quests::evaluate::{GettingStartedActions, QualifyingMatch};

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct QuestClaimRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub quest_id: String,
    pub period_kind: String,
    pub period_id: String,
    pub season_id: Option<i32>,
    pub reward_points: i32,
    pub catalog_version: i32,
    pub claimed_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewQuestClaim {
    pub user_id: UserId,
    pub quest_id: String,
    pub period_kind: String,
    pub period_id: String,
    pub season_id: Option<i32>,
    pub reward_points: i32,
    pub catalog_version: i32,
}

#[derive(Debug, sqlx::FromRow)]
struct MatchRow {
    game_id: String,
    finished_at: DateTime<Utc>,
    is_winner: bool,
    entry_micro: i64,
    creator_id: Uuid,
    player_count: i32,
    opponents: Vec<Uuid>,
}

#[derive(Debug, sqlx::FromRow)]
struct GsRow {
    username_set: bool,
    hosted: bool,
    joined: bool,
    won: bool,
}

#[derive(Debug, sqlx::FromRow)]
struct LeaderboardRow {
    user_id: Uuid,
    points: i64,
    total_matches: i32,
    total_wins: i32,
    total_pnl: i64,
    username: Option<String>,
    display_name: Option<String>,
    avatar_url: Option<String>,
}

impl LeaderboardRow {
    fn into_entry(self, rank: u32) -> LeaderboardEntry {
        let win_rate_bps = if self.total_matches <= 0 {
            0
        } else {
            ((self.total_wins as i64 * 10_000) / self.total_matches as i64) as i32
        };
        LeaderboardEntry {
            rank,
            user_id: UserId(self.user_id),
            points: self.points,
            total_matches: self.total_matches,
            total_wins: self.total_wins,
            total_pnl: self.total_pnl,
            win_rate_bps,
            username: self.username,
            display_name: self.display_name,
            avatar_url: self.avatar_url,
        }
    }
}

pub struct PgQuestRepo {
    pool: PgPool,
}

impl PgQuestRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn qualifying_matches(
        &self,
        user_id: UserId,
        since: DateTime<Utc>,
    ) -> AppResult<Vec<QualifyingMatch>> {
        qualifying_matches(&self.pool, user_id, since).await
    }

    pub async fn getting_started_actions(
        &self,
        user_id: UserId,
    ) -> AppResult<GettingStartedActions> {
        getting_started_actions(&self.pool, user_id).await
    }

    pub async fn claims_for_user(&self, user_id: UserId) -> AppResult<Vec<QuestClaimRow>> {
        claims_for_user(&self.pool, user_id).await
    }

    pub async fn successful_referral_count(&self, user_id: UserId) -> AppResult<i64> {
        successful_referral_count(&self.pool, user_id).await
    }

    pub async fn daily_claim_count(
        &self,
        user_id: UserId,
        period_ids: &[String],
    ) -> AppResult<i64> {
        daily_claim_count(&self.pool, user_id, period_ids).await
    }

    pub async fn season_claim_count(&self, user_id: UserId, season_id: i32) -> AppResult<i64> {
        season_claim_count(&self.pool, user_id, season_id).await
    }

    pub async fn maybe_stamp_getting_started(&self, user_id: UserId) -> AppResult<bool> {
        maybe_stamp_getting_started(&self.pool, user_id).await
    }

    pub async fn insert_claim(
        tx: &mut Transaction<'_, Postgres>,
        claim: &NewQuestClaim,
    ) -> AppResult<Option<QuestClaimRow>> {
        let row = sqlx::query_as::<_, QuestClaimRow>(
            r#"
            INSERT INTO quest_claims (
                user_id, quest_id, period_kind, period_id, season_id,
                reward_points, catalog_version
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            ON CONFLICT (user_id, quest_id, period_id) DO NOTHING
            RETURNING id, user_id, quest_id, period_kind, period_id, season_id,
                      reward_points, catalog_version, claimed_at
            "#,
        )
        .bind(claim.user_id.as_uuid())
        .bind(&claim.quest_id)
        .bind(&claim.period_kind)
        .bind(&claim.period_id)
        .bind(claim.season_id)
        .bind(claim.reward_points)
        .bind(claim.catalog_version)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|err| AppError::Internal(err.into()))?;
        Ok(row)
    }

    pub async fn get_claim(
        tx: &mut Transaction<'_, Postgres>,
        user_id: UserId,
        quest_id: &str,
        period_id: &str,
    ) -> AppResult<Option<QuestClaimRow>> {
        sqlx::query_as::<_, QuestClaimRow>(
            r#"
            SELECT id, user_id, quest_id, period_kind, period_id, season_id,
                   reward_points, catalog_version, claimed_at
            FROM quest_claims
            WHERE user_id = $1 AND quest_id = $2 AND period_id = $3
            "#,
        )
        .bind(user_id.as_uuid())
        .bind(quest_id)
        .bind(period_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|err| AppError::Internal(err.into()))
    }

    pub async fn leaderboard_quests(
        &self,
        season_id: SeasonId,
        limit: i64,
        offset: i64,
    ) -> AppResult<(Vec<LeaderboardEntry>, i64)> {
        let total = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COUNT(DISTINCT user_id)::bigint
            FROM quest_claims
            WHERE season_id = $1
            "#,
        )
        .bind(season_id.as_i32())
        .fetch_one(&self.pool)
        .await
        .map_err(|err| AppError::Internal(err.into()))?;

        let rows = sqlx::query_as::<_, LeaderboardRow>(
            r#"
            SELECT
                u.id AS user_id,
                COALESCE(SUM(c.reward_points), 0)::bigint AS points,
                0::int AS total_matches,
                0::int AS total_wins,
                0::bigint AS total_pnl,
                u.username,
                u.display_name,
                u.avatar_url
            FROM quest_claims c
            JOIN users u ON u.id = c.user_id
            WHERE c.season_id = $1
            GROUP BY u.id, u.username, u.display_name, u.avatar_url
            ORDER BY points DESC, u.id
            LIMIT $2 OFFSET $3
            "#,
        )
        .bind(season_id.as_i32())
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .map_err(|err| AppError::Internal(err.into()))?;

        let items = rows
            .into_iter()
            .enumerate()
            .map(|(i, row)| row.into_entry((offset as u32) + (i as u32) + 1))
            .collect();
        Ok((items, total))
    }

    pub async fn leaderboard_all(
        &self,
        season_id: SeasonId,
        limit: i64,
        offset: i64,
    ) -> AppResult<(Vec<LeaderboardEntry>, i64)> {
        let total = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COUNT(*)::bigint FROM (
                SELECT user_id FROM user_game_stats WHERE season_id = $1
                UNION
                SELECT user_id FROM quest_claims WHERE season_id = $1
            ) ids
            "#,
        )
        .bind(season_id.as_i32())
        .fetch_one(&self.pool)
        .await
        .map_err(|err| AppError::Internal(err.into()))?;

        let rows = sqlx::query_as::<_, LeaderboardRow>(
            r#"
            WITH game AS (
                SELECT user_id,
                       SUM(points) AS points,
                       SUM(total_matches)::int AS total_matches,
                       SUM(total_wins)::int AS total_wins,
                       SUM(total_pnl) AS total_pnl
                FROM user_game_stats
                WHERE season_id = $1
                GROUP BY user_id
            ),
            quest AS (
                SELECT user_id, SUM(reward_points)::bigint AS points
                FROM quest_claims
                WHERE season_id = $1
                GROUP BY user_id
            )
            SELECT
                u.id AS user_id,
                (COALESCE(g.points, 0) + COALESCE(q.points, 0))::bigint AS points,
                COALESCE(g.total_matches, 0)::int AS total_matches,
                COALESCE(g.total_wins, 0)::int AS total_wins,
                COALESCE(g.total_pnl, 0)::bigint AS total_pnl,
                u.username,
                u.display_name,
                u.avatar_url
            FROM users u
            JOIN (
                SELECT user_id FROM game
                UNION
                SELECT user_id FROM quest
            ) ids ON ids.user_id = u.id
            LEFT JOIN game g ON g.user_id = u.id
            LEFT JOIN quest q ON q.user_id = u.id
            ORDER BY points DESC, u.id
            LIMIT $2 OFFSET $3
            "#,
        )
        .bind(season_id.as_i32())
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .map_err(|err| AppError::Internal(err.into()))?;

        let items = rows
            .into_iter()
            .enumerate()
            .map(|(i, row)| row.into_entry((offset as u32) + (i as u32) + 1))
            .collect();
        Ok((items, total))
    }
}

pub async fn qualifying_matches<'e, E>(
    exec: E,
    user_id: UserId,
    since: DateTime<Utc>,
) -> AppResult<Vec<QualifyingMatch>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query_as::<_, MatchRow>(
        r#"
        SELECT
            m.game_id,
            m.finished_at,
            mp.is_winner,
            mp.entry_micro,
            l.creator_id,
            m.player_count,
            COALESCE(
                ARRAY_AGG(op.user_id) FILTER (WHERE op.user_id <> $1),
                '{}'
            ) AS opponents
        FROM match_players mp
        JOIN matches m ON m.id = mp.match_id
        JOIN lobbies l ON l.id = m.lobby_id
        JOIN match_players op ON op.match_id = m.id
        WHERE mp.user_id = $1
          AND m.player_count >= 2
          AND m.finished_at >= $2
        GROUP BY m.id, m.game_id, m.finished_at, mp.is_winner, mp.entry_micro,
                 l.creator_id, m.player_count
        "#,
    )
    .bind(user_id.as_uuid())
    .bind(since)
    .fetch_all(exec)
    .await
    .map_err(|err| AppError::Internal(err.into()))?;

    Ok(rows
        .into_iter()
        .map(|row| QualifyingMatch {
            game_id: row.game_id,
            finished_at: row.finished_at,
            is_winner: row.is_winner,
            entry_micro: row.entry_micro,
            creator_id: row.creator_id,
            player_count: row.player_count,
            opponents: row.opponents,
        })
        .collect())
}

pub async fn getting_started_actions<'e, E>(
    exec: E,
    user_id: UserId,
) -> AppResult<GettingStartedActions>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query_as::<_, GsRow>(
        r#"
        SELECT
            (u.username IS NOT NULL) AS username_set,
            EXISTS (
                SELECT 1
                FROM match_players mp
                JOIN matches m ON m.id = mp.match_id
                JOIN lobbies l ON l.id = m.lobby_id
                WHERE mp.user_id = u.id
                  AND m.player_count >= 2
                  AND l.creator_id = u.id
            ) AS hosted,
            EXISTS (
                SELECT 1
                FROM match_players mp
                JOIN matches m ON m.id = mp.match_id
                JOIN lobbies l ON l.id = m.lobby_id
                WHERE mp.user_id = u.id
                  AND m.player_count >= 2
                  AND l.creator_id <> u.id
            ) AS joined,
            EXISTS (
                SELECT 1
                FROM match_players mp
                JOIN matches m ON m.id = mp.match_id
                WHERE mp.user_id = u.id
                  AND m.player_count >= 2
                  AND mp.is_winner
            ) AS won
        FROM users u
        WHERE u.id = $1
        "#,
    )
    .bind(user_id.as_uuid())
    .fetch_optional(exec)
    .await
    .map_err(|err| AppError::Internal(err.into()))?
    .ok_or(AppError::NotFound("user"))?;

    Ok(GettingStartedActions {
        username_set: row.username_set,
        hosted: row.hosted,
        joined: row.joined,
        won: row.won,
    })
}

pub async fn claims_for_user<'e, E>(exec: E, user_id: UserId) -> AppResult<Vec<QuestClaimRow>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query_as::<_, QuestClaimRow>(
        r#"
        SELECT id, user_id, quest_id, period_kind, period_id, season_id,
               reward_points, catalog_version, claimed_at
        FROM quest_claims
        WHERE user_id = $1
        "#,
    )
    .bind(user_id.as_uuid())
    .fetch_all(exec)
    .await
    .map_err(|err| AppError::Internal(err.into()))
}

pub async fn successful_referral_count<'e, E>(exec: E, user_id: UserId) -> AppResult<i64>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query_scalar(
        r#"
        SELECT COUNT(*)::bigint
        FROM users
        WHERE referred_by_user_id = $1
          AND getting_started_completed_at IS NOT NULL
          AND deleted_at IS NULL
        "#,
    )
    .bind(user_id.as_uuid())
    .fetch_one(exec)
    .await
    .map_err(|err| AppError::Internal(err.into()))
}

pub async fn daily_claim_count<'e, E>(
    exec: E,
    user_id: UserId,
    period_ids: &[String],
) -> AppResult<i64>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    if period_ids.is_empty() {
        return Ok(0);
    }
    sqlx::query_scalar(
        r#"
        SELECT COUNT(*)::bigint
        FROM quest_claims
        WHERE user_id = $1
          AND period_kind = 'daily'
          AND period_id = ANY($2)
        "#,
    )
    .bind(user_id.as_uuid())
    .bind(period_ids)
    .fetch_one(exec)
    .await
    .map_err(|err| AppError::Internal(err.into()))
}

pub async fn season_claim_count<'e, E>(exec: E, user_id: UserId, season_id: i32) -> AppResult<i64>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query_scalar(
        r#"
        SELECT COUNT(*)::bigint
        FROM quest_claims
        WHERE user_id = $1 AND season_id = $2
        "#,
    )
    .bind(user_id.as_uuid())
    .bind(season_id)
    .fetch_one(exec)
    .await
    .map_err(|err| AppError::Internal(err.into()))
}

/// Write-once Getting Started / referral credit when the four actions are true.
pub async fn maybe_stamp_getting_started(pool: &PgPool, user_id: UserId) -> AppResult<bool> {
    let actions = getting_started_actions(pool, user_id).await?;
    if !actions.all_done() {
        return Ok(false);
    }
    let n = sqlx::query(
        r#"
        UPDATE users SET
            getting_started_completed_at = COALESCE(getting_started_completed_at, now()),
            referral_credited_at = CASE
                WHEN referred_by_user_id IS NOT NULL
                    THEN COALESCE(referral_credited_at, now())
                ELSE referral_credited_at
            END,
            updated_at = now()
        WHERE id = $1
          AND deleted_at IS NULL
          AND (
            getting_started_completed_at IS NULL
            OR (referred_by_user_id IS NOT NULL AND referral_credited_at IS NULL)
          )
        "#,
    )
    .bind(user_id.as_uuid())
    .execute(pool)
    .await
    .map_err(|err| AppError::Internal(err.into()))?
    .rows_affected();
    Ok(n > 0)
}
