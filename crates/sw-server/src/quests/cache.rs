//! Redis cache for `GET /quests/me`. Never a source of truth.

use redis::AsyncCommands;
use redis::aio::ConnectionManager;
use sw_domain::UserId;
use tracing::warn;

pub fn cache_key(user_id: UserId) -> String {
    format!("sw:quest:me:{}", user_id.as_uuid())
}

pub async fn get_json(redis: &mut ConnectionManager, user_id: UserId) -> Option<String> {
    let key = cache_key(user_id);
    match redis.get::<_, Option<String>>(&key).await {
        Ok(value) => value,
        Err(err) => {
            warn!(error = %err, "quest cache get failed");
            None
        }
    }
}

pub async fn set_json(
    redis: &mut ConnectionManager,
    user_id: UserId,
    payload: &str,
    ttl_secs: u64,
) {
    let key = cache_key(user_id);
    if let Err(err) = redis
        .set_ex::<_, _, ()>(&key, payload, ttl_secs.max(1))
        .await
    {
        warn!(error = %err, "quest cache set failed");
    }
}

pub async fn invalidate(redis: &mut ConnectionManager, user_id: UserId) {
    let key = cache_key(user_id);
    if let Err(err) = redis.del::<_, ()>(&key).await {
        warn!(error = %err, "quest cache del failed");
    }
}
