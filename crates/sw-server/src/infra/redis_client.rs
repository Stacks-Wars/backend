use anyhow::{Context, Result};
use redis::Client;
use redis::aio::ConnectionManager;
use tracing::info;

/// Open a Redis connection manager and verify connectivity. Required for boot.
pub async fn connect(redis_url: &str) -> Result<ConnectionManager> {
    let client = Client::open(redis_url).context("parse REDIS_URL")?;
    let mut manager = ConnectionManager::new(client)
        .await
        .context("connect to redis")?;

    let pong: String = redis::cmd("PING")
        .query_async(&mut manager)
        .await
        .context("ping redis")?;

    if !pong.eq_ignore_ascii_case("PONG") {
        anyhow::bail!("unexpected redis PING response: {pong}");
    }

    info!("redis connection manager ready");
    Ok(manager)
}
