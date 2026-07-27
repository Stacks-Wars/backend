use axum::routing::{get, post};
use axum::Router;

use crate::error::{AppError, AppResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/health-detail", get(health_detail))
        .route("/seasons", post(create_season))
        .route("/games/reload", post(reload_games))
}

async fn health_detail() -> AppResult<()> {
    Err(AppError::NotImplemented("admin health detail"))
}

async fn create_season() -> AppResult<()> {
    Err(AppError::NotImplemented("admin create season"))
}

async fn reload_games() -> AppResult<()> {
    Err(AppError::NotImplemented("admin reload games"))
}
