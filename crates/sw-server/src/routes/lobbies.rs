use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sw_domain::{
    GameId, Lobby, LobbyId, LobbyState, PlayerState, UserId,
};
use uuid::Uuid;

use crate::data::lobbies::{generate_unique_lobby_path, PgLobbyRepo};
use crate::data::lobby_runtime::{LobbyStateRepo, PlayerStateRepo};
use crate::data::users::PgUserRepo;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_lobbies).post(create_lobby))
        .route("/by-path/{path}", get(get_lobby_by_path))
        .route("/{lobby_id}", get(get_lobby))
        .route("/{lobby_id}/join", post(join_lobby))
        .route("/{lobby_id}/leave", post(leave_lobby))
        .route("/{lobby_id}/ready", post(set_ready))
        .route("/{lobby_id}/start", post(start_lobby))
}

fn require_internal_secret(headers: &HeaderMap, expected: &str) -> AppResult<()> {
    let provided = headers
        .get("x-internal-secret")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if provided.is_empty() || provided != expected {
        return Err(AppError::Unauthorized("invalid internal secret"));
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateLobbyBody {
    name: String,
    description: Option<String>,
    game_id: String,
    creator_id: Uuid,
    #[serde(default)]
    is_private: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LobbyResponse {
    lobby: Lobby,
    state: Option<LobbyState>,
    players: Vec<PlayerState>,
}

async fn list_lobbies() -> AppResult<()> {
    Err(AppError::NotImplemented("list lobbies"))
}

async fn create_lobby(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CreateLobbyBody>,
) -> AppResult<Json<LobbyResponse>> {
    require_internal_secret(&headers, &state.config.internal_api_secret)?;

    let name = body.name.trim().to_owned();
    if name.is_empty() || name.len() > 80 {
        return Err(AppError::BadRequest(
            "name must be 1–80 characters".into(),
        ));
    }

    let game_id = GameId::new(body.game_id).map_err(|e| AppError::BadRequest(e.to_string()))?;
    if !state.games.contains(&game_id) {
        return Err(AppError::NotFound("game not registered"));
    }

    let creator_id = UserId::from(body.creator_id);
    let users = PgUserRepo::new(state.db.clone());
    let creator = users
        .get_by_id(creator_id)
        .await?
        .ok_or(AppError::NotFound("creator user not found"))?;

    let lobbies = PgLobbyRepo::new(state.db.clone());
    let path = generate_unique_lobby_path(&lobbies).await?;
    let now = Utc::now();
    let lobby_id = LobbyId::new();

    let lobby = Lobby {
        id: lobby_id,
        path: path.clone(),
        name,
        description: body
            .description
            .map(|d| d.trim().to_owned())
            .filter(|d| !d.is_empty()),
        game_id,
        creator_id,
        entry_amount: None,
        current_amount: None,
        contract_address: None,
        is_private: body.is_private,
        is_sponsored: false,
        status: sw_domain::LobbyStatus::Waiting,
        created_at: now,
        updated_at: now,
        participants: vec![creator_id],
    };

    lobbies.insert(&lobby).await?;

    let lobby_state = LobbyState::new(lobby_id, 1);
    LobbyStateRepo::new(state.redis.clone())
        .set(&lobby_state)
        .await?;

    let player = PlayerState::creator(creator_id, creator.username, creator.display_name);
    PlayerStateRepo::new(state.redis.clone())
        .set(lobby_id, &player)
        .await?;

    Ok(Json(LobbyResponse {
        lobby,
        state: Some(lobby_state),
        players: vec![player],
    }))
}

async fn get_lobby_by_path(
    State(state): State<AppState>,
    Path(path): Path<String>,
) -> AppResult<Json<LobbyResponse>> {
    let lobbies = PgLobbyRepo::new(state.db.clone());
    let lobby = lobbies
        .get_by_path(&path)
        .await?
        .ok_or(AppError::NotFound("lobby not found"))?;

    let lobby_state = LobbyStateRepo::new(state.redis.clone())
        .get(lobby.id)
        .await?;
    let players = PlayerStateRepo::new(state.redis.clone())
        .list(lobby.id)
        .await?;

    Ok(Json(LobbyResponse {
        lobby,
        state: lobby_state,
        players,
    }))
}

async fn get_lobby(
    State(state): State<AppState>,
    Path(lobby_id): Path<Uuid>,
) -> AppResult<Json<LobbyResponse>> {
    let lobbies = PgLobbyRepo::new(state.db.clone());
    let lobby = lobbies
        .get_by_id(LobbyId::from(lobby_id))
        .await?
        .ok_or(AppError::NotFound("lobby not found"))?;

    let lobby_state = LobbyStateRepo::new(state.redis.clone())
        .get(lobby.id)
        .await?;
    let players = PlayerStateRepo::new(state.redis.clone())
        .list(lobby.id)
        .await?;

    Ok(Json(LobbyResponse {
        lobby,
        state: lobby_state,
        players,
    }))
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
