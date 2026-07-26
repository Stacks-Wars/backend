use axum::routing::get;
use axum::Router;

use crate::error::{AppError, AppResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_users))
        .route("/{user_id}", get(get_user))
        .route("/{user_id}/stats", get(user_stats))
}

async fn list_users() -> AppResult<()> {
    Err(AppError::NotImplemented("list users"))
}

async fn get_user() -> AppResult<()> {
    Err(AppError::NotImplemented("get user"))
}

async fn user_stats() -> AppResult<()> {
    Err(AppError::NotImplemented("user stats"))
}
