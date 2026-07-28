use sqlx::PgPool;
use sw_domain::{usdcx_to_micro, GameId, LeaderboardEntry, SeasonId, UserId};
use uuid::Uuid;

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone)]
pub struct RecordResultInput {
    pub user_id: UserId,
    pub game_id: GameId,
    pub season_id: SeasonId,
    pub points: i64,
    pub is_winner: bool,
    pub prize_dollars: Option<f64>,
    pub entry_dollars: Option<f64>,
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

pub struct PgStatsRepo {
    pool: PgPool,
}

impl PgStatsRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Upsert per-(user, game, season) stats after a match result.
    pub async fn record_result(&self, input: RecordResultInput) -> AppResult<()> {
        let wins_delta: i32 = if input.is_winner { 1 } else { 0 };
        let prize_micro = input.prize_dollars.map(usdcx_to_micro).unwrap_or(0);
        let entry_micro = input.entry_dollars.map(usdcx_to_micro).unwrap_or(0);
        let pnl_delta = prize_micro - entry_micro;

        sqlx::query(
            r#"
            INSERT INTO user_game_stats (
                user_id, game_id, season_id, points, total_matches, total_wins, total_pnl
            )
            VALUES ($1, $2, $3, $4, 1, $5, $6)
            ON CONFLICT (user_id, game_id, season_id) DO UPDATE SET
                points = user_game_stats.points + EXCLUDED.points,
                total_matches = user_game_stats.total_matches + 1,
                total_wins = user_game_stats.total_wins + EXCLUDED.total_wins,
                total_pnl = user_game_stats.total_pnl + EXCLUDED.total_pnl,
                updated_at = now()
            "#,
        )
        .bind(input.user_id.as_uuid())
        .bind(input.game_id.as_str())
        .bind(input.season_id.as_i32())
        .bind(input.points)
        .bind(wins_delta)
        .bind(pnl_delta)
        .execute(&self.pool)
        .await
        .map_err(|err| AppError::Internal(err.into()))?;

        Ok(())
    }

    pub async fn leaderboard_overall(
        &self,
        season_id: SeasonId,
        limit: i64,
        offset: i64,
    ) -> AppResult<(Vec<LeaderboardEntry>, i64)> {
        let total = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COUNT(DISTINCT user_id)::bigint
            FROM user_game_stats
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
                COALESCE(SUM(s.points), 0)::bigint AS points,
                COALESCE(SUM(s.total_matches), 0)::int AS total_matches,
                COALESCE(SUM(s.total_wins), 0)::int AS total_wins,
                COALESCE(SUM(s.total_pnl), 0)::bigint AS total_pnl,
                u.username,
                u.display_name,
                u.avatar_url
            FROM user_game_stats s
            JOIN users u ON u.id = s.user_id
            WHERE s.season_id = $1
            GROUP BY u.id, u.username, u.display_name, u.avatar_url
            ORDER BY points DESC, total_wins DESC, u.id
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

    pub async fn leaderboard_by_game(
        &self,
        season_id: SeasonId,
        game_id: &GameId,
        limit: i64,
        offset: i64,
    ) -> AppResult<(Vec<LeaderboardEntry>, i64)> {
        let total = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COUNT(*)::bigint
            FROM user_game_stats
            WHERE season_id = $1 AND game_id = $2
            "#,
        )
        .bind(season_id.as_i32())
        .bind(game_id.as_str())
        .fetch_one(&self.pool)
        .await
        .map_err(|err| AppError::Internal(err.into()))?;

        let rows = sqlx::query_as::<_, LeaderboardRow>(
            r#"
            SELECT
                u.id AS user_id,
                s.points,
                s.total_matches,
                s.total_wins,
                s.total_pnl,
                u.username,
                u.display_name,
                u.avatar_url
            FROM user_game_stats s
            JOIN users u ON u.id = s.user_id
            WHERE s.season_id = $1 AND s.game_id = $2
            ORDER BY s.points DESC, s.total_wins DESC, u.id
            LIMIT $3 OFFSET $4
            "#,
        )
        .bind(season_id.as_i32())
        .bind(game_id.as_str())
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
