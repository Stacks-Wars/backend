//! One module per scheduled job.

pub mod lobby_free_ttl;
pub mod quest_nudge;
pub mod season;

use std::sync::Arc;

use anyhow::{Context, Result};
use sw_plugin::GameRegistry;
use sw_server::config::Config;
use sw_server::infra::redis_client;
use sw_server::state::AppState;

use crate::connect_db;

/// Build the server's shared state for jobs that publish over WebSocket or send
/// push. Requires the full server environment (app URL, internal secret, Hiro /
/// Helius keys, VAPID keys) in addition to `DATABASE_URL` and `REDIS_URL`.
///
/// The game registry is intentionally empty: these jobs never start a match.
pub async fn app_state() -> Result<AppState> {
    let config = Config::from_env().context("load server config")?;
    let db = connect_db().await?;
    let redis = redis_client::connect(&config.redis_url)
        .await
        .context("connect redis")?;
    Ok(AppState::new(
        config,
        db,
        redis,
        Arc::new(GameRegistry::new()),
    ))
}
