//! Daily quest reminder fan-out.
//!
//! `quest_nudge::start` already claims one send slot per user per UTC day, so a
//! retried or duplicate run is harmless.

use anyhow::Result;
use tracing::info;

use sw_server::services::quest_nudge;

pub async fn run() -> Result<()> {
    let state = super::app_state().await?;
    let result = quest_nudge::start(state).await?;
    info!(
        period_id = %result.period_id,
        targeted = result.targeted,
        "daily quest nudge dispatched"
    );
    Ok(())
}
