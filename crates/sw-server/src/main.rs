use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Context;
use sw_plugin::GameRegistry;
use tracing::info;

use sw_server::config::Config;
use sw_server::data::seasons::PgSeasonRepo;
use sw_server::games;
use sw_server::infra::{postgres, redis_client};
use sw_server::routes;
use sw_server::state::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    init_tracing();

    let config = Config::from_env().context("load config")?;
    info!(
        host = %config.host,
        port = config.port,
        "starting stacks wars server"
    );

    let db = postgres::connect(&config.database_url)
        .await
        .context("postgres")?;
    PgSeasonRepo::new(db.clone())
        .seed_year_to_current_quarter_if_empty()
        .await
        .context("seed seasons")?;
    let redis = redis_client::connect(&config.redis_url)
        .await
        .context("redis")?;

    let game_registry = GameRegistry::new();
    games::register_games(&game_registry).context("register games")?;
    info!(registered = game_registry.len(), "game plugins registered");

    let state = AppState::new(config.clone(), db, redis, Arc::new(game_registry));
    sw_server::services::telegram::spawn_bot(state.clone());

    // Free waiting lobbies older than 24h can be purged without on-chain work.
    // Paid stale lobbies are refunded + expired by the Next cron (/api/cron/lobby-ttl).
    {
        let janitor = state.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(15 * 60));
            loop {
                ticker.tick().await;
                match sw_server::services::lobby_ttl::expire_free_stale_lobbies(&janitor).await {
                    Ok(0) => {}
                    Ok(n) => tracing::info!(expired = n, "expired free stale lobbies"),
                    Err(err) => tracing::warn!(error = %err, "free lobby TTL janitor failed"),
                }
            }
        });
    }

    let app = routes::router(state);

    let addr = SocketAddr::from((config.host, config.port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("bind {addr}"))?;

    info!(%addr, "listening");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .context("serve http")?;

    Ok(())
}

fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,sw_server=debug".into()),
        )
        .with_target(true)
        .compact()
        .init();
}
