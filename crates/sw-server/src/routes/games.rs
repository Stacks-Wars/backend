use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use sw_domain::{GameCatalogEntry, GameId};

use crate::error::{AppError, AppResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_games))
        .route("/{game_id}", get(get_game))
}

/// Real catalog listing backed by the in-process plugin registry.
async fn list_games(State(state): State<AppState>) -> Json<Vec<GameCatalogEntry>> {
    Json(state.games.list_catalog())
}

async fn get_game(
    State(state): State<AppState>,
    Path(game_id): Path<String>,
) -> AppResult<Json<GameCatalogEntry>> {
    let id = GameId::new(game_id);
    state
        .games
        .get(&id)
        .map(|factory| Json(factory.catalog_entry()))
        .ok_or(AppError::NotFound("game not registered"))
}
