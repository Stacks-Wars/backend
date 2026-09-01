use std::sync::Arc;

use redis::aio::ConnectionManager;
use sqlx::PgPool;
use sw_domain::UserId;
use tracing::warn;

use crate::data::quest_claims::PgQuestRepo;
use crate::services::realtime;
use crate::ws::{SessionManager, SubscriptionManager};

/// After a successful match-history write: cache DEL, Getting Started stamp, WS.
/// Safe to retry. Does not touch `user_game_stats`.
pub fn spawn_after_match(
    db: PgPool,
    redis: ConnectionManager,
    sessions: Arc<SessionManager>,
    subscriptions: Arc<SubscriptionManager>,
    user_ids: Vec<UserId>,
) {
    tokio::spawn(async move {
        let repo = PgQuestRepo::new(db);
        for user_id in user_ids {
            let mut redis = redis.clone();
            crate::quests::cache::invalidate(&mut redis, user_id).await;
            if let Err(err) = repo.maybe_stamp_getting_started(user_id).await {
                warn!(user_id = %user_id, error = %err, "quest getting-started stamp failed");
            }
            realtime::publish_quest_updated_raw(&subscriptions, &sessions, user_id);
        }
    });
}
