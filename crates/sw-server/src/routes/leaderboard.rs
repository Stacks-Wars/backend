use axum::routing::get;
use axum::Router;

use crate::error::{AppError, AppResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(global_leaderboard))
        .route("/seasons/{season_id}", get(season_leaderboard))
}

async fn global_leaderboard() -> AppResult<()> {
    Err(AppError::NotImplemented("global leaderboard"))
}

async fn season_leaderboard() -> AppResult<()> {
    Err(AppError::NotImplemented("season leaderboard"))
}
