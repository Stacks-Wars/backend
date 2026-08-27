use axum::extract::{Path, State};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use sw_domain::{LobbyId, SeasonId, UserId};
use uuid::Uuid;

use crate::auth::{AuthUser, InternalSecret};
use crate::config::{USDCX_ASSET_NAME, USDCX_CONTRACT};
use crate::data::lobbies::PgLobbyRepo;
use crate::data::lobby_runtime::PlayerStateRepo;
use crate::data::seasons::{PgSeasonRepo, SeasonRepo, UpdateSeasonInput};
use crate::error::{AppError, AppResult};
use crate::services::hiro::HiroClient;
use crate::services::lobby_ttl::{self, StaleLobby};
use crate::services::realtime;
use crate::services::vault_verify::VaultReader;
use crate::services::wallet_chain::WalletChainService;
use crate::state::AppState;

/// Admin mutations — Write rate tier (still requires admin / internal auth).
pub fn write_router() -> Router<AppState> {
    Router::new()
        .route("/seasons", post(create_season))
        .route("/seasons/{season_id}", put(update_season))
        .route("/lobbies/{lobby_id}/expire-seat", post(expire_seat))
        .route("/lobbies/{lobby_id}/expire", post(expire_lobby))
}

/// Admin reads — Global tier only.
pub fn read_router() -> Router<AppState> {
    Router::new().route("/lobbies/stale", get(list_stale_lobbies))
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
struct ExpireSeatBody {
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

/// Confirm one seat was refunded on-chain (or was free), then drop it from the lobby.
async fn expire_seat(
    State(state): State<AppState>,
    _secret: InternalSecret,
    Path(lobby_id): Path<Uuid>,
    Json(body): Json<ExpireSeatBody>,
) -> AppResult<Json<serde_json::Value>> {
    let lobby_id = LobbyId::from(lobby_id);
    let user_id = UserId::from(body.user_id);
    let lobbies = PgLobbyRepo::new(state.db.clone());
    let lobby = lobbies
        .get_by_id(lobby_id)
        .await?
        .ok_or(AppError::NotFound("lobby not found"))?;

    if lobby.status != sw_domain::LobbyStatus::Waiting {
        return Err(AppError::Conflict("lobby not waiting".into()));
    }
    if !lobby.participants.iter().any(|p| *p == user_id) {
        return Err(AppError::NotFound("seat not in lobby"));
    }

    let paid = if lobby.entry_amount_micro <= 0 {
        0
    } else if lobby.is_sponsored && lobby.creator_id != user_id {
        0
    } else {
        lobby.entry_amount_micro
    };

    // Any vault lobby (entry > 0) must prove the seat left the contract map.
    if lobby.entry_amount_micro > 0 {
        let txid = body
            .vault_txid
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                AppError::BadRequest("vaultTxid required for vault lobby seat".into())
            })?;
        let hiro = HiroClient::new(
            state.config.hiro_api_url.clone(),
            state.config.hiro_api_key.clone(),
            USDCX_CONTRACT,
            USDCX_ASSET_NAME,
            Some(state.config.sw_vault_contract.clone()),
        );
        let reader = VaultReader::new(&hiro, &state.config.sw_vault_contract);
        reader
            .assert_not_joined(&lobby.path, body.address.trim(), txid)
            .await?;
        let _ = WalletChainService::new(state.db.clone(), state.redis.clone(), hiro)
            .refresh_balance(user_id)
            .await;
    }

    lobbies.remove_participant(lobby_id, user_id, paid).await?;
    PlayerStateRepo::new(state.redis.clone())
        .delete(lobby_id, user_id)
        .await
        .ok();

    Ok(Json(serde_json::json!({ "ok": true })))
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
