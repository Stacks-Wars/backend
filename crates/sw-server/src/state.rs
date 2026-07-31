use std::sync::Arc;

use redis::aio::ConnectionManager;
use sqlx::PgPool;
use sw_plugin::GameRegistry;

use crate::config::Config;
use crate::engine::EngineRegistry;
use crate::services::neon_jwt::NeonJwtVerifier;
use crate::services::telegram::TelegramNotifier;
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
    /// Running match actors, keyed by lobby.
    pub engines: Arc<EngineRegistry>,
    pub jwt: Arc<NeonJwtVerifier>,
    pub telegram: Arc<TelegramNotifier>,
}

impl AppState {
    pub fn new(
        config: Config,
        db: PgPool,
        redis: ConnectionManager,
        games: Arc<GameRegistry>,
    ) -> Self {
        let jwt = config.jwt_verifier();
        let telegram = TelegramNotifier::from_config(&config);
        Self {
            config: Arc::new(config),
            db,
            redis,
            games,
            sessions: SessionManager::arc(),
            subscriptions: SubscriptionManager::arc(),
            engines: EngineRegistry::arc(),
            jwt,
            telegram,
        }
    }
}
