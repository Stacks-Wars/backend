//! `sw-cron <job> [--loop]`
//!
//! Default behaviour is one-shot: run the job, close the pool, exit. That is
//! what Railway's cron services expect, and the exit code is the result.
//!
//! Locally, `--loop` keeps the process alive on the job's natural cadence so the
//! same code path can be exercised without a scheduler.

use std::process::ExitCode;

use anyhow::Result;
use tracing_subscriber::EnvFilter;

use sw_cron::Job;

#[tokio::main]
async fn main() -> ExitCode {
    dotenvy::dotenv().ok();
    init_tracing();

    let mut args = std::env::args().skip(1);
    let Some(raw_job) = args.next() else {
        eprintln!("{}", usage());
        return ExitCode::from(2);
    };
    if raw_job == "--help" || raw_job == "-h" {
        println!("{}", usage());
        return ExitCode::SUCCESS;
    }

    let Some(job) = Job::parse(&raw_job) else {
        eprintln!("unknown job `{raw_job}`\n\n{}", usage());
        return ExitCode::from(2);
    };
    let looped = args.any(|arg| arg == "--loop");

    match run(job, looped).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            // Railway records the failed run; keep the chain for the log.
            eprintln!("{job} job failed: {err:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run(job: Job, looped: bool) -> Result<()> {
    tracing::info!(job = %job, looped, "starting job");

    if !looped {
        return sw_cron::run(job).await;
    }

    let interval = job.interval();
    tracing::info!(job = %job, interval_secs = interval.as_secs(), "looping");
    let mut ticker = tokio::time::interval(interval);
    loop {
        ticker.tick().await;
        // A failed tick must not kill the loop: report it and wait for the next.
        if let Err(err) = sw_cron::run(job).await {
            tracing::error!(job = %job, error = ?err, "job run failed");
        }
    }
}

fn usage() -> String {
    let individual: Vec<&str> = Job::EACH.iter().map(|job| job.name()).collect();
    format!(
        "sw-cron — scheduled jobs\n\n\
         usage: sw-cron <{}|{}> [--loop]\n\n\
         `all` runs every due job in one pass, which is what the Railway service\n\
         runs hourly; naming a job runs just that one. Runs once and exits unless\n\
         --loop is passed.",
        Job::All.name(),
        individual.join("|")
    )
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,sw_cron=debug,sw_server=info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jobs_parse_case_insensitively() {
        assert_eq!(Job::parse("all"), Some(Job::All));
        assert_eq!(Job::parse(" ALL "), Some(Job::All));
        assert_eq!(Job::parse("season"), Some(Job::Season));
        assert_eq!(Job::parse(" SEASON "), Some(Job::Season));
        assert_eq!(Job::parse("quest-nudge"), Some(Job::QuestNudge));
        assert_eq!(Job::parse("lobby-free-ttl"), Some(Job::LobbyFreeTtl));
        assert_eq!(Job::parse("nope"), None);
    }

    /// `all` must not collide with a job name, since both are parsed by string.
    #[test]
    fn job_names_are_unique() {
        let mut names: Vec<&str> = std::iter::once(Job::All)
            .chain(Job::EACH)
            .map(|job| job.name())
            .collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len());
    }

    #[test]
    fn usage_lists_every_job() {
        let usage = usage();
        for job in std::iter::once(Job::All).chain(Job::EACH) {
            assert!(usage.contains(job.name()), "usage omits {}", job.name());
        }
    }

    /// The combined run is the Railway entry point, so it must be the default
    /// shape described in the help output.
    #[test]
    fn every_job_shares_the_hourly_cadence() {
        for job in std::iter::once(Job::All).chain(Job::EACH) {
            assert_eq!(job.interval().as_secs(), 60 * 60, "{}", job.name());
        }
    }
}
