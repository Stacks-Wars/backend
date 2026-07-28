use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{GameId, SeasonId, UserId};

/// One micro-USDCx = 10^-6 USDCx ($1 = 1_000_000).
pub const USDCX_MICROS_PER_UNIT: i64 = 1_000_000;

/// Convert a dollar amount to micro-USDCx (nearest micro).
pub fn usdcx_to_micro(dollars: f64) -> i64 {
    (dollars * USDCX_MICROS_PER_UNIT as f64).round() as i64
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Season {
    pub id: SeasonId,
    pub name: String,
    pub description: Option<String>,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

/// Per-user, per-game, per-season accumulated stats (durable).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserGameStats {
    pub id: Uuid,
    pub user_id: UserId,
    pub game_id: GameId,
    pub season_id: SeasonId,
    pub points: i64,
    pub total_matches: i32,
    pub total_wins: i32,
    /// Net PnL in micro-USDCx.
    pub total_pnl: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl UserGameStats {
    /// Win rate in basis points (0..=10_000). Zero matches → 0.
    pub fn win_rate_bps(&self) -> i32 {
        if self.total_matches <= 0 {
            return 0;
        }
        ((self.total_wins as i64 * 10_000) / self.total_matches as i64) as i32
    }
}

/// Leaderboard row (query join over stats + users).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LeaderboardEntry {
    pub rank: u32,
    pub user_id: UserId,
    pub points: i64,
    pub total_matches: i32,
    pub total_wins: i32,
    pub total_pnl: i64,
    /// Win rate in basis points (0..=10_000).
    pub win_rate_bps: i32,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
}
