//! Scheduled one-shot jobs for Stacks Wars.
//!
//! Railway runs each job as its own service with a cron schedule: the container
//! starts, the job runs to completion, and the process exits. Railway skips the
//! next run while a previous one is still active, so nothing here may block
//! indefinitely.
//!
//! Jobs that only need Postgres deliberately avoid building a full `AppState`,
//! which keeps the required environment small (see `backend/README.md`).

pub mod jobs;

use std::fmt;
use std::time::Duration;

use anyhow::Result;
use sqlx::PgPool;

/// A unit of scheduled work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Job {
    /// Create the upcoming season shortly before the current one ends.
    Season,
    /// Daily quest reminder fan-out.
    QuestNudge,
    /// Expire free waiting lobbies older than their TTL.
    LobbyFreeTtl,
}

impl Job {
    pub const ALL: [Job; 3] = [Job::Season, Job::QuestNudge, Job::LobbyFreeTtl];

    pub fn name(self) -> &'static str {
        match self {
            Job::Season => "season",
            Job::QuestNudge => "quest-nudge",
            Job::LobbyFreeTtl => "lobby-free-ttl",
        }
    }

    /// Suggested cadence, also used as the `--loop` interval locally.
    pub fn interval(self) -> Duration {
        match self {
            // Cheap, and it means a skipped run still has several chances
            // inside the season lead window.
            Job::Season => Duration::from_secs(60 * 60),
            Job::QuestNudge => Duration::from_secs(60 * 60 * 24),
            Job::LobbyFreeTtl => Duration::from_secs(60 * 15),
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|job| job.name().eq_ignore_ascii_case(raw.trim()))
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

/// Run one job to completion.
pub async fn run(job: Job) -> Result<()> {
    match job {
        Job::Season => jobs::season::run().await,
        Job::QuestNudge => jobs::quest_nudge::run().await,
        Job::LobbyFreeTtl => jobs::lobby_free_ttl::run().await,
    }
}
