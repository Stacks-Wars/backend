use axum::routing::{get, post};
use axum::Router;

use crate::error::{AppError, AppResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/login", post(login))
        .route("/logout", post(logout))
        .route("/me", get(me))
}

async fn login() -> AppResult<()> {
    Err(AppError::NotImplemented("auth login"))
}

async fn logout() -> AppResult<()> {
    Err(AppError::NotImplemented("auth logout"))
}

async fn me() -> AppResult<()> {
    Err(AppError::NotImplemented("auth me"))
}
