use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use sw_domain::{GameId, GameMetadata};

use crate::error::{AppError, AppResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_games))
        .route("/{game_id}", get(get_game))
}

/// Catalog listing backed by the in-process plugin registry (empty until games register).
async fn list_games(State(state): State<AppState>) -> Json<Vec<GameMetadata>> {
    Json(state.games.list_metadata())
}

async fn get_game(
    State(state): State<AppState>,
    Path(game_id): Path<String>,
) -> AppResult<Json<GameMetadata>> {
    let id = GameId::new(game_id).map_err(|err| AppError::BadRequest(err.to_string()))?;
    state
        .games
        .get(&id)
        .map(|factory| Json(factory.metadata()))
        .ok_or(AppError::NotFound("game not registered"))
}
