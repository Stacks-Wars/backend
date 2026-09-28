//! Scheduled one-shot jobs for Stacks Wars.
//!
//! Railway runs this as one hourly service: the container starts, every due job
//! runs, and the process exits. Railway skips the next run while a previous one
//! is still active, so nothing here may block indefinitely.
//!
//! Each job decides for itself whether it has work, which is what lets one
//! schedule serve three cadences (see `Job::All`). Individual jobs stay
//! addressable by name for local runs and for debugging a single concern.
//!
//! The combined run deliberately avoids building a full `AppState` per job; see
//! `jobs::run_all`.

pub mod jobs;

use std::fmt;
use std::time::Duration;

use anyhow::Result;
use sqlx::PgPool;

/// A unit of scheduled work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Job {
    /// Every due job in one pass. This is the Railway entry point.
    All,
    /// Create the upcoming season shortly before the current one ends.
    Season,
    /// Daily quest reminder fan-out, inside its UTC window.
    QuestNudge,
    /// Expire free waiting lobbies older than their TTL.
    LobbyFreeTtl,
}

impl Job {
    /// The individual jobs, in the order the combined run executes them.
    pub const EACH: [Job; 3] = [Job::Season, Job::LobbyFreeTtl, Job::QuestNudge];

    pub fn name(self) -> &'static str {
        match self {
            Job::All => "all",
            Job::Season => "season",
            Job::QuestNudge => "quest-nudge",
            Job::LobbyFreeTtl => "lobby-free-ttl",
        }
    }

    /// Suggested cadence, also used as the `--loop` interval locally.
    ///
    /// Every job is gated on its own condition, so all four share the hourly
    /// tick the Railway service runs on.
    pub fn interval(self) -> Duration {
        Duration::from_secs(60 * 60)
    }

    pub fn parse(raw: &str) -> Option<Self> {
        let raw = raw.trim();
        if Job::All.name().eq_ignore_ascii_case(raw) {
            return Some(Job::All);
        }
        Self::EACH
            .into_iter()
            .find(|job| job.name().eq_ignore_ascii_case(raw))
    }
}

impl fmt::Display for Job {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Connect to Postgres and apply migrations.
///
/// Safe to run alongside the API: SQLx takes an advisory lock for the migration
/// step, so two services booting at once do not race.
pub async fn connect_db() -> Result<PgPool> {
    let url = sw_server::config::database_url_from_env()?;
    sw_server::infra::postgres::connect(&url).await
}

/// Run one job to completion. `Job::All` runs every job; anything else runs just
/// that one, which is what local usage and targeted debugging want.
pub async fn run(job: Job) -> Result<()> {
    match job {
        Job::All => jobs::run_all().await,
        Job::Season => jobs::season::run().await,
        Job::QuestNudge => jobs::quest_nudge::run().await,
        Job::LobbyFreeTtl => jobs::lobby_free_ttl::run().await,
    }
}
