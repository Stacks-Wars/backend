//! Daily quest reminder fan-out.
//!
//! The schedule is hourly rather than `0 10 * * *`, so the job opens a UTC
//! window and the first run inside it does the work. `quest_nudge::start` claims
//! one send slot per user per UTC day, so the remaining runs in the window find
//! nothing to send and are no-ops.
//!
//! Checking the clock *before* calling `start` is what keeps the nudge at 10:00
//! rather than midnight: claiming slots is a side effect of the call.

use anyhow::Result;
use chrono::{DateTime, Timelike, Utc};
use tracing::info;

use sw_server::services::quest_nudge;
use sw_server::state::AppState;

/// First UTC hour that may nudge. Matches the old fixed `0 10 * * *` schedule.
pub const NUDGE_FROM_HOUR: u32 = 10;
/// Last UTC hour that may nudge. The window exists so a skipped or delayed run
/// still delivers while the daily quests are actionable, without nudging people
/// in the small hours.
pub const NUDGE_UNTIL_HOUR: u32 = 21;

/// Whether a run at `now` should attempt the daily nudge.
pub fn nudge_due(now: DateTime<Utc>) -> bool {
    (NUDGE_FROM_HOUR..=NUDGE_UNTIL_HOUR).contains(&now.hour())
}

pub async fn run() -> Result<()> {
    let state = super::app_state().await?;
    tick(&state).await
}

/// Shares the caller's `AppState`; used by the combined hourly run.
pub async fn tick(state: &AppState) -> Result<()> {
    let now = Utc::now();
    if !nudge_due(now) {
        info!(
            hour = now.hour(),
            window = %format!("{NUDGE_FROM_HOUR}-{NUDGE_UNTIL_HOUR}"),
            "quest nudge outside its window"
        );
        return Ok(());
    }

    let result = quest_nudge::start(state.clone()).await?;
    if result.targeted == 0 {
        // Usual case for the later runs in the window: today's slots are claimed.
        info!(period_id = %result.period_id, "quest nudge already sent or no recipients");
    } else {
        info!(
            period_id = %result.period_id,
            targeted = result.targeted,
            "daily quest nudge dispatched"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 28, hour, 0, 0).unwrap()
    }

    #[test]
    fn before_the_window_is_not_due() {
        assert!(!nudge_due(at(0)));
        assert!(!nudge_due(at(9)));
    }

    #[test]
    fn inside_the_window_is_due() {
        assert!(nudge_due(at(10)));
        assert!(nudge_due(at(15)));
        assert!(nudge_due(at(21)));
    }

    #[test]
    fn after_the_window_is_not_due() {
        assert!(!nudge_due(at(22)));
        assert!(!nudge_due(at(23)));
    }

    /// The service ticks on the hour, but a delayed run can land mid-hour.
    #[test]
    fn minute_within_the_hour_does_not_matter() {
        let mid_hour = Utc.with_ymd_and_hms(2026, 9, 28, 10, 37, 12).unwrap();
        assert!(nudge_due(mid_hour));
    }
}
