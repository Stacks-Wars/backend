use anyhow::{Context, Result};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tracing::info;

/// Open a Postgres pool and verify connectivity. Required for boot.
pub async fn connect(database_url: &str) -> Result<PgPool> {
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(database_url)
        .await
        .context("connect to postgres")?;

    sqlx::query("SELECT 1")
        .execute(&pool)
        .await
        .context("ping postgres")?;

    info!("postgres pool ready");
    Ok(pool)
}
