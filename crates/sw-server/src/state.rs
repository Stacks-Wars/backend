use std::sync::Arc;

use redis::aio::ConnectionManager;
use sqlx::PgPool;
use sw_plugin::GameRegistry;

use crate::config::Config;
use crate::ws::{SessionManager, SubscriptionManager};

/// Shared application state injected into HTTP / WS handlers.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub db: PgPool,
    pub redis: ConnectionManager,
    pub games: Arc<GameRegistry>,
    pub sessions: Arc<SessionManager>,
    pub subscriptions: Arc<SubscriptionManager>,
}

impl AppState {
    pub fn new(
        config: Config,
        db: PgPool,
        redis: ConnectionManager,
        games: Arc<GameRegistry>,
    ) -> Self {
        Self {
            config: Arc::new(config),
            db,
            redis,
            games,
            sessions: SessionManager::arc(),
            subscriptions: SubscriptionManager::arc(),
        }
    }
}
