use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use sw_domain::{GameId, GameMetadata};

use crate::data::lobbies::PgLobbyRepo;
use crate::data::matches::{PgMatchRepo, RecentMatch};
use crate::error::{AppError, AppResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_games))
        .route("/activity", get(list_activity))
        .route("/recent-matches", get(list_recent_matches))
        .route("/{game_id}", get(get_game))
        .route("/{game_id}/activity", get(get_game_activity))
}

/// Live counters shown on game cards. Games with no open lobbies report zeros.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GameActivityResponse {
    game_id: String,
    waiting_lobbies: i64,
    live_lobbies: i64,
    active_players: i64,
    open_pot_micro: i64,
}

async fn activity_map(state: &AppState) -> AppResult<HashMap<String, GameActivityResponse>> {
    let rows = PgLobbyRepo::new(state.db.clone()).game_activity().await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            (
                row.game_id.clone(),
                GameActivityResponse {
                    game_id: row.game_id,
                    waiting_lobbies: row.waiting_lobbies,
                    live_lobbies: row.live_lobbies,
                    active_players: row.active_players,
                    open_pot_micro: row.open_pot_micro,
                },
            )
        })
        .collect())
}

fn zeroed(game_id: &str) -> GameActivityResponse {
    GameActivityResponse {
        game_id: game_id.to_owned(),
        waiting_lobbies: 0,
        live_lobbies: 0,
        active_players: 0,
        open_pot_micro: 0,
    }
}

async fn list_activity(
    State(state): State<AppState>,
) -> AppResult<Json<Vec<GameActivityResponse>>> {
    let mut activity = activity_map(&state).await?;
    let items = state
        .games
        .list_metadata()
        .into_iter()
        .map(|meta| {
            let id = meta.id.as_str();
            activity.remove(id).unwrap_or_else(|| zeroed(id))
        })
        .collect();
    Ok(Json(items))
}

async fn get_game_activity(
    State(state): State<AppState>,
    Path(game_id): Path<String>,
) -> AppResult<Json<GameActivityResponse>> {
    let mut activity = activity_map(&state).await?;
    Ok(Json(
        activity
            .remove(&game_id)
            .unwrap_or_else(|| zeroed(&game_id)),
    ))
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecentQuery {
    limit: Option<i64>,
}

async fn list_recent_matches(
    State(state): State<AppState>,
    Query(params): Query<RecentQuery>,
) -> AppResult<Json<Vec<RecentMatch>>> {
    let items = PgMatchRepo::new(state.db.clone())
        .recent(params.limit.unwrap_or(12).clamp(1, 50))
        .await?;
    Ok(Json(items))
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
