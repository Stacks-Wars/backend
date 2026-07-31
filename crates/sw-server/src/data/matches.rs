use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
use sw_domain::{LobbyId, MatchId, SeasonId, UserId};
use uuid::Uuid;

use crate::error::{AppError, AppResult};

/// A finished match plus the calling user's outcome in it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchHistoryItem {
    pub match_id: Uuid,
    pub lobby_id: Uuid,
    pub lobby_path: String,
    pub game_id: String,
    pub pot_micro: i64,
    pub entry_amount_micro: i64,
    pub player_count: i32,
    pub finished_at: DateTime<Utc>,
    pub rank: Option<i32>,
    pub is_winner: bool,
    pub prize_micro: i64,
    pub wars_point: i64,
}

#[derive(Debug, sqlx::FromRow)]
struct MatchHistoryRow {
    match_id: Uuid,
    lobby_id: Uuid,
    lobby_path: String,
    game_id: String,
    pot_micro: i64,
    entry_amount_micro: i64,
    player_count: i32,
    finished_at: DateTime<Utc>,
    rank: Option<i32>,
    is_winner: bool,
    prize_micro: i64,
    wars_point: i64,
}

/// One player's line in a settled match.
#[derive(Debug, Clone)]
pub struct MatchPlayerRecord {
    pub user_id: UserId,
    pub rank: Option<i32>,
    pub is_winner: bool,
    pub prize_micro: i64,
    pub entry_micro: i64,
    pub wars_point: i64,
}

#[derive(Debug, Clone)]
pub struct MatchRecord {
    pub match_id: MatchId,
    pub lobby_id: LobbyId,
    pub lobby_path: String,
    pub game_id: String,
    pub season_id: Option<SeasonId>,
    pub pot_micro: i64,
    pub entry_amount_micro: i64,
    pub started_at: Option<DateTime<Utc>>,
    pub players: Vec<MatchPlayerRecord>,
}

pub struct PgMatchRepo {
    pool: PgPool,
}

impl PgMatchRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Write a settled match and its player rows. Idempotent per lobby.
    pub async fn record(&self, record: &MatchRecord) -> AppResult<()> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| AppError::Internal(e.into()))?;

        sqlx::query(
            r#"
            INSERT INTO matches (
                id, lobby_id, lobby_path, game_id, season_id,
                pot_micro, entry_amount_micro, player_count, started_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            ON CONFLICT (lobby_id) DO NOTHING
            "#,
        )
        .bind(record.match_id.as_uuid())
        .bind(record.lobby_id.as_uuid())
        .bind(&record.lobby_path)
        .bind(&record.game_id)
        .bind(record.season_id.map(|s| s.as_i32()))
        .bind(record.pot_micro)
        .bind(record.entry_amount_micro)
        .bind(record.players.len() as i32)
        .bind(record.started_at)
        .execute(&mut *tx)
        .await
        .map_err(|e| AppError::Internal(e.into()))?;

        for player in &record.players {
            sqlx::query(
                r#"
                INSERT INTO match_players (
                    match_id, user_id, rank, is_winner,
                    prize_micro, entry_micro, wars_point
                ) VALUES ($1, $2, $3, $4, $5, $6, $7)
                ON CONFLICT (match_id, user_id) DO UPDATE SET
                    rank = EXCLUDED.rank,
                    is_winner = EXCLUDED.is_winner,
                    prize_micro = EXCLUDED.prize_micro,
                    entry_micro = EXCLUDED.entry_micro,
                    wars_point = EXCLUDED.wars_point
                "#,
            )
            .bind(record.match_id.as_uuid())
            .bind(player.user_id.as_uuid())
            .bind(player.rank)
            .bind(player.is_winner)
            .bind(player.prize_micro)
            .bind(player.entry_micro)
            .bind(player.wars_point)
            .execute(&mut *tx)
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        }

        tx.commit().await.map_err(|e| AppError::Internal(e.into()))?;
        Ok(())
    }

    /// Finished match for a lobby, if settle already wrote history.
    pub async fn get_by_lobby(
        &self,
        lobby_id: LobbyId,
    ) -> AppResult<Option<(Uuid, String, i64, Vec<(Uuid, Option<i32>, bool, i64, i64)>)>> {
        let row: Option<(Uuid, String, i64)> = sqlx::query_as(
            r#"
            SELECT id, lobby_path, pot_micro
            FROM matches
            WHERE lobby_id = $1
            "#,
        )
        .bind(lobby_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::Internal(e.into()))?;

        let Some((match_id, lobby_path, pot_micro)) = row else {
            return Ok(None);
        };

        let players: Vec<(Uuid, Option<i32>, bool, i64, i64)> = sqlx::query_as(
            r#"
            SELECT user_id, rank, is_winner, prize_micro, wars_point
            FROM match_players
            WHERE match_id = $1
            ORDER BY rank ASC NULLS LAST
            "#,
        )
        .bind(match_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::Internal(e.into()))?;

        Ok(Some((match_id, lobby_path, pot_micro, players)))
    }

    pub async fn history_for_user(
        &self,
        user_id: UserId,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<MatchHistoryItem>> {
        let rows = sqlx::query_as::<_, MatchHistoryRow>(
            r#"
            SELECT m.id AS match_id, m.lobby_id, m.lobby_path, m.game_id,
                   m.pot_micro, m.entry_amount_micro, m.player_count, m.finished_at,
                   mp.rank, mp.is_winner, mp.prize_micro, mp.wars_point
            FROM match_players mp
            JOIN matches m ON m.id = mp.match_id
            WHERE mp.user_id = $1
            ORDER BY m.finished_at DESC
            LIMIT $2 OFFSET $3
            "#,
        )
        .bind(user_id.as_uuid())
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::Internal(e.into()))?;

        Ok(rows.into_iter().map(into_item).collect())
    }

    /// Games a user plays most, by match count.
    pub async fn favourite_games(
        &self,
        user_id: UserId,
        limit: i64,
    ) -> AppResult<Vec<(String, i64, i64)>> {
        let rows: Vec<(String, i64, i64)> = sqlx::query_as(
            r#"
            SELECT m.game_id,
                   COUNT(*) AS matches,
                   COUNT(*) FILTER (WHERE mp.is_winner) AS wins
            FROM match_players mp
            JOIN matches m ON m.id = mp.match_id
            WHERE mp.user_id = $1
            GROUP BY m.game_id
            ORDER BY matches DESC
            LIMIT $2
            "#,
        )
        .bind(user_id.as_uuid())
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::Internal(e.into()))?;
        Ok(rows)
    }

    /// Platform-wide recent results, optionally scoped to one game.
    pub async fn recent(
        &self,
        game_id: Option<&str>,
        limit: i64,
    ) -> AppResult<Vec<RecentMatch>> {
        let rows = sqlx::query_as::<_, RecentMatchRow>(
            r#"
            SELECT m.id AS match_id, m.lobby_path, m.game_id, m.pot_micro,
                   m.player_count, m.finished_at,
                   u.id AS winner_id, u.username AS winner_username,
                   u.display_name AS winner_display_name,
                   u.avatar_url AS winner_avatar_url,
                   mp.prize_micro AS winner_prize_micro
            FROM matches m
            LEFT JOIN match_players mp
                ON mp.match_id = m.id AND mp.is_winner = true
            LEFT JOIN users u ON u.id = mp.user_id
            WHERE ($1::TEXT IS NULL OR m.game_id = $1)
            ORDER BY m.finished_at DESC
            LIMIT $2
            "#,
        )
        .bind(game_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::Internal(e.into()))?;

        Ok(rows
            .into_iter()
            .map(|r| RecentMatch {
                match_id: r.match_id,
                lobby_path: r.lobby_path,
                game_id: r.game_id,
                pot_micro: r.pot_micro,
                player_count: r.player_count,
                finished_at: r.finished_at,
                winner_id: r.winner_id,
                winner_username: r.winner_username,
                winner_display_name: r.winner_display_name,
                winner_avatar_url: r.winner_avatar_url,
                winner_prize_micro: r.winner_prize_micro.unwrap_or(0),
            })
            .collect())
    }

    /// Aggregate lifetime numbers for a profile header.
    pub async fn lifetime_totals(&self, user_id: UserId) -> AppResult<LifetimeTotals> {
        let row: Option<LifetimeTotalsRow> = sqlx::query_as(
            r#"
            SELECT COUNT(*) AS total_matches,
                   COUNT(*) FILTER (WHERE is_winner) AS total_wins,
                   COALESCE(SUM(prize_micro), 0)::bigint AS total_winnings_micro,
                   COALESCE(SUM(prize_micro - entry_micro), 0)::bigint AS total_pnl_micro,
                   COALESCE(SUM(wars_point), 0)::bigint AS total_points
            FROM match_players
            WHERE user_id = $1
            "#,
        )
        .bind(user_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::Internal(e.into()))?;

        Ok(row.map(Into::into).unwrap_or_default())
    }
}

fn into_item(r: MatchHistoryRow) -> MatchHistoryItem {
    MatchHistoryItem {
        match_id: r.match_id,
        lobby_id: r.lobby_id,
        lobby_path: r.lobby_path,
        game_id: r.game_id,
        pot_micro: r.pot_micro,
        entry_amount_micro: r.entry_amount_micro,
        player_count: r.player_count,
        finished_at: r.finished_at,
        rank: r.rank,
        is_winner: r.is_winner,
        prize_micro: r.prize_micro,
        wars_point: r.wars_point,
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentMatch {
    pub match_id: Uuid,
    pub lobby_path: String,
    pub game_id: String,
    pub pot_micro: i64,
    pub player_count: i32,
    pub finished_at: DateTime<Utc>,
    pub winner_id: Option<Uuid>,
    pub winner_username: Option<String>,
    pub winner_display_name: Option<String>,
    pub winner_avatar_url: Option<String>,
    pub winner_prize_micro: i64,
}

#[derive(Debug, sqlx::FromRow)]
struct RecentMatchRow {
    match_id: Uuid,
    lobby_path: String,
    game_id: String,
    pot_micro: i64,
    player_count: i32,
    finished_at: DateTime<Utc>,
    winner_id: Option<Uuid>,
    winner_username: Option<String>,
    winner_display_name: Option<String>,
    winner_avatar_url: Option<String>,
    winner_prize_micro: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LifetimeTotals {
    pub total_matches: i64,
    pub total_wins: i64,
    pub total_winnings_micro: i64,
    pub total_pnl_micro: i64,
    pub total_points: i64,
}

#[derive(Debug, sqlx::FromRow)]
struct LifetimeTotalsRow {
    total_matches: i64,
    total_wins: i64,
    total_winnings_micro: i64,
    total_pnl_micro: i64,
    total_points: i64,
}

impl From<LifetimeTotalsRow> for LifetimeTotals {
    fn from(r: LifetimeTotalsRow) -> Self {
        Self {
            total_matches: r.total_matches,
            total_wins: r.total_wins,
            total_winnings_micro: r.total_winnings_micro,
            total_pnl_micro: r.total_pnl_micro,
            total_points: r.total_points,
        }
    }
}
