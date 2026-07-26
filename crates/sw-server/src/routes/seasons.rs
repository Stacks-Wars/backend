use axum::routing::get;
use axum::Router;

use crate::error::{AppError, AppResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_seasons))
        .route("/current", get(current_season))
        .route("/{season_id}", get(get_season))
}

async fn list_seasons() -> AppResult<()> {
    Err(AppError::NotImplemented("list seasons"))
}

async fn current_season() -> AppResult<()> {
    Err(AppError::NotImplemented("current season"))
}

async fn get_season() -> AppResult<()> {
    Err(AppError::NotImplemented("get season"))
}
