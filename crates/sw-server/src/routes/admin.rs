use axum::extract::{Path, State};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use sw_domain::{LobbyId, SeasonId, UserId};
use uuid::Uuid;

use crate::auth::{AuthUser, InternalSecret};
use crate::data::lobbies::PgLobbyRepo;
use crate::data::seasons::{PgSeasonRepo, SeasonRepo, UpdateSeasonInput};
use crate::error::{AppError, AppResult};
use crate::services::lobby_ttl::{self, StaleLobby};
use crate::services::quest_nudge;
use crate::services::realtime;
use crate::state::AppState;

/// Admin mutations — Write rate tier (still requires admin / internal auth).
pub fn write_router() -> Router<AppState> {
    Router::new()
        .route("/seasons", post(create_season))
        .route("/seasons/{season_id}", put(update_season))
        .route("/lobbies/{lobby_id}/expire-seat", post(expire_seat))
        .route("/lobbies/{lobby_id}/expire", post(expire_lobby))
        .route("/lobbies/{lobby_id}/void-seat", post(void_seat))
        .route("/lobbies/{lobby_id}/void", post(void_lobby))
        .route("/quests/daily-nudge", post(daily_quest_nudge))
}

/// Admin reads — Global tier only.
pub fn read_router() -> Router<AppState> {
    Router::new()
        .route("/lobbies/stale", get(list_stale_lobbies))
        .route("/lobbies/stale-live", get(list_orphaned_lobbies))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateSeasonBody {
    name: String,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateSeasonBody {
    name: String,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SeatRefundBody {
    user_id: Uuid,
    address: String,
    /// Omit / empty when the seat was free (sponsored guest or free lobby).
    #[serde(default)]
    vault_txid: Option<String>,
}

/// Create the next quarterly season. Dates are computed server-side.
async fn create_season(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<CreateSeasonBody>,
) -> AppResult<Json<sw_domain::Season>> {
    auth.require_admin(&state.config.admin_emails)?;

    let season = PgSeasonRepo::new(state.db.clone())
        .create_next_quarter(body.name, body.description)
        .await?;

    Ok(Json(season))
}

/// Update season name / description only (dates stay fixed).
async fn update_season(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(season_id): Path<i32>,
    Json(body): Json<UpdateSeasonBody>,
) -> AppResult<Json<sw_domain::Season>> {
    auth.require_admin(&state.config.admin_emails)?;

    let season = PgSeasonRepo::new(state.db.clone())
        .update(
            SeasonId(season_id),
            UpdateSeasonInput {
                name: body.name,
                description: body.description,
            },
        )
        .await?;

    Ok(Json(season))
}

async fn list_stale_lobbies(
    State(state): State<AppState>,
    _secret: InternalSecret,
) -> AppResult<Json<Vec<StaleLobby>>> {
    Ok(Json(lobby_ttl::list_stale_waiting(&state).await?))
}

/// Live lobbies whose match actor died with the server. Same seat shape as the
/// waiting list, so the cron refunds both with one loop.
async fn list_orphaned_lobbies(
    State(state): State<AppState>,
    _secret: InternalSecret,
) -> AppResult<Json<Vec<StaleLobby>>> {
    Ok(Json(lobby_ttl::list_orphaned_live(&state).await?))
}

async fn daily_quest_nudge(
    State(state): State<AppState>,
    _secret: InternalSecret,
) -> AppResult<Json<quest_nudge::DailyNudgeResult>> {
    Ok(Json(quest_nudge::start(state).await?))
}

/// Confirm one seat was refunded on-chain (or was free), then drop it from the lobby.
async fn expire_seat(
    State(state): State<AppState>,
    _secret: InternalSecret,
    Path(lobby_id): Path<Uuid>,
    Json(body): Json<SeatRefundBody>,
) -> AppResult<Json<serde_json::Value>> {
    let lobby_id = LobbyId::from(lobby_id);
    let lobby = PgLobbyRepo::new(state.db.clone())
        .get_by_id(lobby_id)
        .await?
        .ok_or(AppError::NotFound("lobby not found"))?;

    if lobby.status != sw_domain::LobbyStatus::Waiting {
        return Err(AppError::Conflict("lobby not waiting".into()));
    }

    lobby_ttl::clear_seat(
        &state,
        lobby_id,
        UserId::from(body.user_id),
        &body.address,
        body.vault_txid.as_deref(),
    )
    .await?;

    Ok(Json(serde_json::json!({ "ok": true })))
}

/// Confirm one seat of an orphaned live lobby was refunded, then drop it.
async fn void_seat(
    State(state): State<AppState>,
    _secret: InternalSecret,
    Path(lobby_id): Path<Uuid>,
    Json(body): Json<SeatRefundBody>,
) -> AppResult<Json<serde_json::Value>> {
    let lobby_id = LobbyId::from(lobby_id);
    let lobby = PgLobbyRepo::new(state.db.clone())
        .get_by_id(lobby_id)
        .await?
        .ok_or(AppError::NotFound("lobby not found"))?;

    if !matches!(
        lobby.status,
        sw_domain::LobbyStatus::Starting | sw_domain::LobbyStatus::InProgress
    ) {
        return Err(AppError::Conflict("lobby not live".into()));
    }
    if state.engines.is_running(lobby_id) {
        return Err(AppError::Conflict("match is still running".into()));
    }

    lobby_ttl::clear_seat(
        &state,
        lobby_id,
        UserId::from(body.user_id),
        &body.address,
        body.vault_txid.as_deref(),
    )
    .await?;

    Ok(Json(serde_json::json!({ "ok": true })))
}

/// Finish a live lobby with no running match, once every paid seat is clear.
async fn void_lobby(
    State(state): State<AppState>,
    _secret: InternalSecret,
    Path(lobby_id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    let voided = lobby_ttl::void_lobby(&state, LobbyId::from(lobby_id)).await?;
    Ok(Json(serde_json::json!({
        "ok": true,
        "lobbyId": voided.id,
        "path": voided.path,
    })))
}

/// Delete a waiting lobby after all seats have been cleared (and refunded if paid).
async fn expire_lobby(
    State(state): State<AppState>,
    _secret: InternalSecret,
    Path(lobby_id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    let lobby_id = LobbyId::from(lobby_id);
    let lobby = PgLobbyRepo::new(state.db.clone())
        .get_by_id(lobby_id)
        .await?
        .ok_or(AppError::NotFound("lobby not found"))?;

    if !lobby.participants.is_empty() && lobby.entry_amount_micro > 0 {
        return Err(AppError::Conflict(
            "refund all paid seats before expiring lobby".into(),
        ));
    }

    let expired = lobby_ttl::expire_lobby(&state, lobby_id).await?;
    realtime::publish_game_activity(&state).await;
    Ok(Json(serde_json::json!({
        "ok": true,
        "lobbyId": expired.id,
        "path": expired.path,
    })))
}
