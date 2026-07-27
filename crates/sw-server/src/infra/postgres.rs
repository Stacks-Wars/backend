use anyhow::{Context, Result};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tracing::info;

/// Open a Postgres pool, run migrations, and verify connectivity. Required for boot.
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

    // Resolve from CARGO_MANIFEST_DIR so `cargo run -p sw-server` works from any cwd.
    let migrations_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
    let migrator = sqlx::migrate::Migrator::new(migrations_dir)
        .await
        .context("load migrations")?;
    migrator.run(&pool).await.context("run migrations")?;
    info!("postgres migrations applied");

    info!("postgres pool ready");
    Ok(pool)
}
