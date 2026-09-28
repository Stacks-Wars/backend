//! Expire free waiting lobbies older than their TTL.
//!
//! Paid waiting lobbies are refunded on-chain by the Vercel cron
//! (`/api/cron/lobby-ttl`) because the vault signing keys live in the frontend
//! environment, and orphaned live lobbies are voided from the API process where
//! the in-memory engine registry lives. Only the free sweep belongs here.
//!
//! The TTL itself is the gate: `expire_free_stale_lobbies` only touches lobbies
//! older than 24h, so running this hourly instead of every 15 minutes just means
//! a stale lobby can linger up to an hour longer.

use anyhow::Result;
use tracing::info;

use sw_server::services::lobby_ttl;
use sw_server::state::AppState;

pub async fn run() -> Result<()> {
    let state = super::app_state().await?;
    tick(&state).await
}

/// Shares the caller's `AppState`; used by the combined hourly run.
pub async fn tick(state: &AppState) -> Result<()> {
    match lobby_ttl::expire_free_stale_lobbies(state).await? {
        0 => info!("no free stale lobbies to expire"),
        expired => info!(expired, "expired free stale lobbies"),
    }
    Ok(())
}
