use axum::routing::{get, post};
use axum::Router;

use crate::auth::AuthUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/health-detail", get(health_detail))
        .route("/seasons", post(create_season))
        .route("/games/reload", post(reload_games))
}

async fn health_detail(_user: AuthUser) -> AppResult<()> {
    Err(AppError::NotImplemented("admin health detail"))
}

async fn create_season(_user: AuthUser) -> AppResult<()> {
    Err(AppError::NotImplemented("admin create season"))
}

async fn reload_games(_user: AuthUser) -> AppResult<()> {
    Err(AppError::NotImplemented("admin reload games"))
}
