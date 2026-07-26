use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

use crate::state::AppState;

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub service: &'static str,
    pub postgres: &'static str,
    pub redis: &'static str,
    pub games_registered: usize,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/health", get(health))
}

pub async fn root() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "service": "stacks-wars",
        "message": "Stacks Wars backend"
    }))
}

pub async fn health(State(state): State<AppState>) -> (StatusCode, Json<HealthResponse>) {
    let postgres_ok = sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&state.db)
        .await
        .is_ok_and(|n| n == 1);

    let mut redis = state.redis.clone();
    let redis_ok = redis::cmd("PING")
        .query_async::<String>(&mut redis)
        .await
        .is_ok_and(|pong| pong.eq_ignore_ascii_case("PONG"));

    let healthy = postgres_ok && redis_ok;
    let status = if healthy {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    (
        status,
        Json(HealthResponse {
            status: if healthy { "ok" } else { "degraded" },
            service: "stacks-wars",
            postgres: if postgres_ok { "up" } else { "down" },
            redis: if redis_ok { "up" } else { "down" },
            games_registered: state.games.len(),
        }),
    )
}
