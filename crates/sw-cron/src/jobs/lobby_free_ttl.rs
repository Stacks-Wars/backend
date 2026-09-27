//! Expire free waiting lobbies older than their TTL.
//!
//! Paid waiting lobbies are refunded on-chain by the Vercel cron
//! (`/api/cron/lobby-ttl`) because the vault signing keys live in the frontend
//! environment, and orphaned live lobbies are voided from the API process where
//! the in-memory engine registry lives. Only the free sweep belongs here.

use anyhow::Result;
use tracing::{info, warn};

use sw_server::services::lobby_ttl;

pub async fn run() -> Result<()> {
    let state = super::app_state().await?;
    match lobby_ttl::expire_free_stale_lobbies(&state).await {
        Ok(0) => info!("no free stale lobbies to expire"),
        Ok(expired) => info!(expired, "expired free stale lobbies"),
        Err(err) => {
            warn!(error = %err, "free lobby TTL sweep failed");
            return Err(err.into());
        }
    }
    Ok(())
}
