use axum::routing::{get, post};
use axum::Router;

use crate::error::{AppError, AppResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_lobbies).post(create_lobby))
        .route("/{lobby_id}", get(get_lobby))
        .route("/{lobby_id}/join", post(join_lobby))
        .route("/{lobby_id}/leave", post(leave_lobby))
        .route("/{lobby_id}/ready", post(set_ready))
        .route("/{lobby_id}/start", post(start_lobby))
}

async fn list_lobbies() -> AppResult<()> {
    Err(AppError::NotImplemented("list lobbies"))
}

async fn create_lobby() -> AppResult<()> {
    Err(AppError::NotImplemented("create lobby"))
}

async fn get_lobby() -> AppResult<()> {
    Err(AppError::NotImplemented("get lobby"))
}

async fn join_lobby() -> AppResult<()> {
    Err(AppError::NotImplemented("join lobby"))
}

async fn leave_lobby() -> AppResult<()> {
    Err(AppError::NotImplemented("leave lobby"))
}

async fn set_ready() -> AppResult<()> {
    Err(AppError::NotImplemented("lobby ready"))
}

async fn start_lobby() -> AppResult<()> {
    Err(AppError::NotImplemented("start lobby"))
}
