//! Create the upcoming season shortly before the current one ends.
//!
//! The season list only ever holds one future season. Running hourly with a
//! multi-hour lead means the successor exists before the boundary, and every run
//! after the first is a cheap no-op. If the service is down through the whole
//! lead window, the next run after the boundary still creates the season — late,
//! but self-healing.

use anyhow::{Context, Result};
use chrono::Duration;
use tracing::info;

use sw_server::data::seasons::{PgSeasonRepo, SeasonRepo};

/// How long before the current season ends its successor is created.
pub const SEASON_LEAD: Duration = Duration::hours(6);

pub async fn run() -> Result<()> {
    let db = crate::connect_db().await?;
    let repo = PgSeasonRepo::new(db);

    match repo.ensure_upcoming_season(SEASON_LEAD).await? {
        Some(season) => info!(
            season_id = season.id.0,
            name = %season.name,
            starts_at = %season.starts_at,
            ends_at = %season.ends_at,
            "upcoming season ensured"
        ),
        None => info!(
            lead_hours = SEASON_LEAD.num_hours(),
            "no upcoming season due"
        ),
    }

    let latest = repo.latest().await.context("read latest season")?;
    if let Some(latest) = latest {
        info!(
            season_id = latest.id.0,
            name = %latest.name,
            ends_at = %latest.ends_at,
            "latest season"
        );
    }
    Ok(())
}
