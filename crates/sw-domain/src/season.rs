use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{SeasonId, UserId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Season {
    pub id: SeasonId,
    pub name: String,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub active: bool,
}

/// Accumulated wars points for a user within a season.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WarsPoints {
    pub season_id: SeasonId,
    pub user_id: UserId,
    pub points: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaderboardEntry {
    pub rank: u32,
    pub user_id: UserId,
    pub display_name: String,
    pub points: i64,
}
