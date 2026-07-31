use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sw_domain::{
    GameId, JoinRequestState, Lobby, LobbyId, LobbyState, LobbyStatus, PlayerState, UserId,
};
use sw_plugin::EngineContext;
use uuid::Uuid;

use crate::config::{MIN_ENTRY_MICRO, USDCX_ASSET_NAME, USDCX_CONTRACT};
use crate::auth::AuthUser;
use crate::data::join_requests::{JoinRequest, JoinRequestRepo};
use crate::data::lobbies::{generate_unique_lobby_path, LobbyQuery, PgLobbyRepo};
use crate::data::seat_holds::SeatHoldRepo;
use crate::services::realtime;
use crate::data::lobby_runtime::{LobbyStateRepo, PlayerStateRepo};
use crate::data::users::PgUserRepo;
use crate::error::{AppError, AppResult};
use crate::host::ServerGameHost;
use crate::services::hiro::HiroClient;
use crate::services::vault_verify::VaultReader;
use crate::services::wallet_chain::WalletChainService;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_lobbies).post(create_lobby))
        .route("/allocate-path", post(allocate_path))
        .route("/by-path/{path}", get(get_lobby_by_path))
        .route("/{lobby_id}", get(get_lobby))
        .route("/{lobby_id}/reserve-seat", post(reserve_seat).delete(release_seat))
        .route("/{lobby_id}/join", post(join_lobby))
        .route("/{lobby_id}/join-requests", post(create_join_request))
        .route(
            "/{lobby_id}/join-requests/{user_id}/approve",
            post(approve_join_request),
        )
        .route(
            "/{lobby_id}/join-requests/{user_id}/reject",
            post(reject_join_request),
        )
        .route("/{lobby_id}/leave", post(leave_lobby))
        .route("/{lobby_id}/kick", post(kick_lobby_player))
        .route(
            "/{lobby_id}/players/{user_id}/vault-address",
            get(get_kick_target_address),
        )
        .route("/{lobby_id}/ready", post(set_ready))
        .route("/{lobby_id}/start", post(start_lobby))
        .route("/{lobby_id}/vault-claim", post(confirm_vault_claim))
}

fn require_vault_txid(provided: Option<&str>, needs_vault: bool) -> AppResult<Option<String>> {
    if !needs_vault {
        return Ok(None);
    }
    let txid = provided
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            AppError::BadRequest("vaultTxid required for paid/sponsored vault lobbies".into())
        })?;
    Ok(Some(txid.to_owned()))
}

fn hiro_client(state: &AppState) -> HiroClient {
    HiroClient::new(
        state.config.hiro_api_url.clone(),
        state.config.hiro_api_key.clone(),
        USDCX_CONTRACT,
        USDCX_ASSET_NAME,
        Some(state.config.sw_vault_contract.clone()),
    )
}

fn vault_reader<'a>(state: &'a AppState, hiro: &'a HiroClient) -> VaultReader<'a> {
    VaultReader::new(hiro, &state.config.sw_vault_contract)
}

async fn custodial_address(state: &AppState, user_id: UserId) -> AppResult<String> {
    PgUserRepo::new(state.db.clone())
        .get_custodial_wallet(user_id)
        .await?
        .map(|w| w.stx_address)
        .ok_or(AppError::NotFound("custodial wallet not found"))
}

/// Post-vault / money path: re-read Hiro, rewrite Redis, push to the user topic.
async fn refresh_user_balance(state: &AppState, user_id: UserId) {
    let svc = WalletChainService::new(
        state.db.clone(),
        state.redis.clone(),
        hiro_client(state),
    );
    if let Ok(bal) = svc.refresh_balance(user_id).await {
        realtime::publish_wallet_balance(
            state,
            user_id,
            json!({
                "availableMicro": bal.available_micro,
                "stxAddress": bal.stx_address,
            }),
        );
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateLobbyBody {
    name: String,
    description: Option<String>,
    game_id: String,
    #[serde(default)]
    is_private: bool,
    #[serde(default)]
    is_sponsored: bool,
    #[serde(default)]
    entry_amount_micro: i64,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    vault_txid: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UserActionBody {
    #[serde(default)]
    vault_txid: Option<String>,
    /// Ready toggle; `None` means "ready up".
    #[serde(default)]
    ready: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct KickBody {
    target_user_id: Uuid,
    #[serde(default)]
    vault_txid: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VaultClaimBody {
    amount_micro: i64,
    #[allow(dead_code)]
    nonce: u64,
    vault_txid: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LobbyResponse {
    lobby: Lobby,
    state: Option<LobbyState>,
    players: Vec<PlayerState>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListLobbiesQuery {
    game_id: Option<String>,
    /// Comma separated: `waiting,starting,inProgress,finished`.
    status: Option<String>,
    creator_id: Option<Uuid>,
    /// `paid` / `free`; omit for both.
    entry: Option<String>,
    min_players: Option<i32>,
    max_players: Option<i32>,
    include_private: Option<bool>,
    limit: Option<i64>,
    offset: Option<i64>,
}

fn parse_status_list(raw: &str) -> Option<Vec<LobbyStatus>> {
    let parsed: Vec<LobbyStatus> = raw
        .split(',')
        .filter_map(|s| match s.trim() {
            "waiting" => Some(LobbyStatus::Waiting),
            "starting" => Some(LobbyStatus::Starting),
            "inProgress" | "in_progress" => Some(LobbyStatus::InProgress),
            "finished" => Some(LobbyStatus::Finished),
            _ => None,
        })
        .collect();
    (!parsed.is_empty()).then_some(parsed)
}

async fn list_lobbies(
    State(state): State<AppState>,
    Query(params): Query<ListLobbiesQuery>,
) -> AppResult<Json<Vec<Lobby>>> {
    let game_id = params
        .game_id
        .as_deref()
        .map(GameId::try_from)
        .transpose()
        .map_err(|e| AppError::BadRequest(e.to_string()))?;

    let query = LobbyQuery {
        game_id,
        statuses: params
            .status
            .as_deref()
            .and_then(parse_status_list)
            .or_else(|| {
                Some(vec![
                    LobbyStatus::Waiting,
                    LobbyStatus::Starting,
                    LobbyStatus::InProgress,
                ])
            }),
        creator_id: params.creator_id.map(UserId::from),
        paid: match params.entry.as_deref() {
            Some("paid") => Some(true),
            Some("free") => Some(false),
            _ => None,
        },
        min_players: params.min_players,
        max_players: params.max_players,
        // Private lobbies are listed with a lock; omit the filter unless a
        // client explicitly asks for public-only (`include_private=false`).
        is_private: match params.include_private {
            Some(false) => Some(false),
            _ => None,
        },
        limit: params.limit.unwrap_or(60).clamp(1, 200),
        offset: params.offset.unwrap_or(0).max(0),
    };

    let items = PgLobbyRepo::new(state.db.clone()).browse(&query).await?;
    Ok(Json(items))
}

async fn create_lobby(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<CreateLobbyBody>,
) -> AppResult<Json<LobbyResponse>> {

    let name = body.name.trim().to_owned();
    if name.is_empty() || name.len() > 80 {
        return Err(AppError::BadRequest(
            "name must be 1–80 characters".into(),
        ));
    }
    let entry = body.entry_amount_micro;
    if entry < 0 {
        return Err(AppError::BadRequest(
            "entryAmountMicro must be >= 0".into(),
        ));
    }
    if entry > 0 && entry < MIN_ENTRY_MICRO {
        return Err(AppError::BadRequest(format!(
            "paid entry must be at least {MIN_ENTRY_MICRO} micro-USDCx ($1)"
        )));
    }

    let game_id = GameId::new(body.game_id).map_err(|e| AppError::BadRequest(e.to_string()))?;
    if !state.games.contains(&game_id) {
        return Err(AppError::NotFound("game not registered"));
    }

    let creator_id = auth.user_id;
    let users = PgUserRepo::new(state.db.clone());
    let creator = users
        .get_by_id(creator_id)
        .await?
        .ok_or(AppError::NotFound("creator user not found"))?;

    let lobbies = PgLobbyRepo::new(state.db.clone());
    let path = match body.path.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(p) => {
            if p.len() > 64
                || !p
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            {
                return Err(AppError::BadRequest(
                    "path must be 1–64 ascii alphanumeric/_/-".into(),
                ));
            }
            if lobbies.path_exists(p).await? {
                return Err(AppError::Conflict("lobby path already taken".into()));
            }
            p.to_owned()
        }
        None => generate_unique_lobby_path(&lobbies).await?,
    };

    let needs_vault = entry > 0;
    let vault_txid = require_vault_txid(body.vault_txid.as_deref(), needs_vault)?;
    if needs_vault {
        let addr = custodial_address(&state, creator_id).await?;
        let hiro = hiro_client(&state);
        let reader = vault_reader(&state, &hiro);
        reader
            .assert_joined(&path, &addr, entry, vault_txid.as_deref().unwrap())
            .await?;
        refresh_user_balance(&state, creator_id).await;
    }

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
        entry_amount_micro: entry,
        pot_micro: entry,
        is_private: body.is_private,
        is_sponsored: body.is_sponsored && entry > 0,
        status: LobbyStatus::Waiting,
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

    realtime::publish_lobby_feed(&state, realtime::LobbyFeedKind::Created, &lobby);
    realtime::publish_game_activity(&state).await;
    state.telegram.notify_lobby_created(&state, &lobby);

    Ok(Json(LobbyResponse {
        lobby,
        state: Some(lobby_state),
        players: vec![player],
    }))
}

/// Fan out a room change to the room, the global browser feed, and the
/// per-game activity counters.
async fn announce_lobby_change(state: &AppState, lobby: &Lobby, reason: &str, notice: Value) {
    realtime::publish_lobby_state(state, lobby.id, reason).await;
    if !notice.is_null() {
        realtime::publish_room_notice(state, lobby.id, notice);
    }
    realtime::publish_lobby_change(state, lobby);
    realtime::publish_game_activity(state).await;
}

async fn lobby_response(state: &AppState, lobby: Lobby) -> AppResult<LobbyResponse> {
    let lobby_state = LobbyStateRepo::new(state.redis.clone())
        .get(lobby.id)
        .await?;
    let players = PlayerStateRepo::new(state.redis.clone())
        .list(lobby.id)
        .await?;
    Ok(LobbyResponse {
        lobby,
        state: lobby_state,
        players,
    })
}

async fn get_lobby_by_path(
    State(state): State<AppState>,
    Path(path): Path<String>,
) -> AppResult<Json<LobbyResponse>> {
    let lobby = PgLobbyRepo::new(state.db.clone())
        .get_by_path(&path)
        .await?
        .ok_or(AppError::NotFound("lobby not found"))?;
    Ok(Json(lobby_response(&state, lobby).await?))
}

async fn get_lobby(
    State(state): State<AppState>,
    Path(lobby_id): Path<Uuid>,
) -> AppResult<Json<LobbyResponse>> {
    let lobby = PgLobbyRepo::new(state.db.clone())
        .get_by_id(LobbyId::from(lobby_id))
        .await?
        .ok_or(AppError::NotFound("lobby not found"))?;
    Ok(Json(lobby_response(&state, lobby).await?))
}

async fn reserve_seat(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(lobby_id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    let lobby_id = LobbyId::from(lobby_id);
    let user_id = auth.user_id;
    let lobby = PgLobbyRepo::new(state.db.clone())
        .get_by_id(lobby_id)
        .await?
        .ok_or(AppError::NotFound("lobby not found"))?;

    if lobby.status != LobbyStatus::Waiting {
        return Err(AppError::Conflict("lobby is not joinable".into()));
    }
    if lobby.participants.iter().any(|p| *p == user_id) {
        return Ok(Json(json!({ "ok": true, "alreadyJoined": true })));
    }
    if lobby.is_private && user_id != lobby.creator_id {
        let jr = JoinRequestRepo::new(state.redis.clone())
            .get(lobby_id, user_id)
            .await?;
        let allowed = matches!(
            jr.as_ref().map(|r| r.state),
            Some(JoinRequestState::Accepted)
        );
        if !allowed {
            return Err(AppError::Unauthorized(
                "private lobby requires an accepted join request".into(),
            ));
        }
    }

    let meta = state
        .games
        .get(&lobby.game_id)
        .ok_or(AppError::NotFound("game not registered"))?
        .metadata();
    let reserved = SeatHoldRepo::new(state.redis.clone())
        .try_reserve(
            lobby_id,
            user_id,
            meta.max_players,
            lobby.participants.len(),
            &lobby.participants,
        )
        .await?;
    if !reserved {
        return Err(AppError::LobbyFull);
    }
    Ok(Json(json!({ "ok": true, "reserved": true })))
}

async fn release_seat(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(lobby_id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    SeatHoldRepo::new(state.redis.clone())
        .release(LobbyId::from(lobby_id), auth.user_id)
        .await?;
    Ok(Json(json!({ "ok": true })))
}

async fn join_lobby(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(lobby_id): Path<Uuid>,
    Json(body): Json<UserActionBody>,
) -> AppResult<Json<LobbyResponse>> {
    let lobby_id = LobbyId::from(lobby_id);
    let user_id = auth.user_id;
    let lobbies = PgLobbyRepo::new(state.db.clone());
    let mut lobby = lobbies
        .get_by_id(lobby_id)
        .await?
        .ok_or(AppError::NotFound("lobby not found"))?;

    if lobby.status != LobbyStatus::Waiting {
        return Err(AppError::Conflict("lobby is not joinable".into()));
    }
    if lobby.participants.iter().any(|p| *p == user_id) {
        return Err(AppError::Conflict("already in lobby".into()));
    }

    if lobby.is_private && user_id != lobby.creator_id {
        let jr = JoinRequestRepo::new(state.redis.clone())
            .get(lobby_id, user_id)
            .await?;
        let allowed = matches!(
            jr.as_ref().map(|r| r.state),
            Some(JoinRequestState::Accepted)
        );
        if !allowed {
            return Err(AppError::Unauthorized(
                "private lobby requires an accepted join request".into(),
            ));
        }
    }

    let meta = state
        .games
        .get(&lobby.game_id)
        .ok_or(AppError::NotFound("game not registered"))?
        .metadata();
    // Re-check / ensure seat hold under the capacity lock before accepting payment proof.
    let seats = SeatHoldRepo::new(state.redis.clone());
    let reserved = seats
        .try_reserve(
            lobby_id,
            user_id,
            meta.max_players,
            lobby.participants.len(),
            &lobby.participants,
        )
        .await?;
    if !reserved {
        return Err(AppError::LobbyFull);
    }

    let user = PgUserRepo::new(state.db.clone())
        .get_by_id(user_id)
        .await?
        .ok_or(AppError::NotFound("user not found"))?;

    let entry = lobby.entry_amount_micro;
    let needs_vault = entry > 0;
    let paid = if !needs_vault {
        0
    } else if lobby.is_sponsored {
        0
    } else {
        entry
    };
    let vault_txid = require_vault_txid(body.vault_txid.as_deref(), needs_vault)?;
    if needs_vault {
        let addr = custodial_address(&state, user_id).await?;
        let hiro = hiro_client(&state);
        let reader = vault_reader(&state, &hiro);
        if let Err(err) = reader
            .assert_joined(&lobby.path, &addr, paid, vault_txid.as_deref().unwrap())
            .await
        {
            let _ = seats.release(lobby_id, user_id).await;
            return Err(err);
        }
        if let Ok(pot) = reader.get_pot(&lobby.path, &addr).await {
            lobby.pot_micro = pot;
        }
        refresh_user_balance(&state, user_id).await;
    }

    let pot_delta = if lobby.is_sponsored { 0 } else { entry };
    if let Err(err) = lobbies
        .add_participant(lobby_id, user_id, pot_delta)
        .await
    {
        let _ = seats.release(lobby_id, user_id).await;
        return Err(err);
    }

    let player = PlayerState::joiner(user_id, user.username, user.display_name);
    PlayerStateRepo::new(state.redis.clone())
        .set(lobby_id, &player)
        .await?;
    let _ = seats.release(lobby_id, user_id).await;

    lobby = lobbies
        .get_by_id(lobby_id)
        .await?
        .ok_or(AppError::NotFound("lobby not found"))?;

    let mut lobby_state = LobbyStateRepo::new(state.redis.clone())
        .get(lobby_id)
        .await?
        .unwrap_or_else(|| LobbyState::new(lobby_id, lobby.participants.len()));
    lobby_state.participant_count = lobby.participants.len();
    LobbyStateRepo::new(state.redis.clone())
        .set(&lobby_state)
        .await?;

    JoinRequestRepo::new(state.redis.clone())
        .delete(lobby_id, user_id)
        .await
        .ok();

    announce_lobby_change(
        &state,
        &lobby,
        "player.joined",
        json!({
            "type": "playerJoined",
            "userId": user_id,
            "username": player.username,
            "displayName": player.display_name,
        }),
    )
    .await;

    Ok(Json(lobby_response(&state, lobby).await?))
}

async fn create_join_request(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(lobby_id): Path<Uuid>,
) -> AppResult<Json<JoinRequest>> {
    let lobby_id = LobbyId::from(lobby_id);
    let user_id = auth.user_id;
    let lobby = PgLobbyRepo::new(state.db.clone())
        .get_by_id(lobby_id)
        .await?
        .ok_or(AppError::NotFound("lobby not found"))?;

    if !lobby.is_private {
        return Err(AppError::BadRequest(
            "join requests are only for private lobbies".into(),
        ));
    }
    if lobby.status != LobbyStatus::Waiting {
        return Err(AppError::Conflict("lobby is not accepting requests".into()));
    }
    if lobby.participants.iter().any(|p| *p == user_id) {
        return Err(AppError::Conflict("already in lobby".into()));
    }
    if user_id == lobby.creator_id {
        return Err(AppError::BadRequest("creator is already in the lobby".into()));
    }

    let user = PgUserRepo::new(state.db.clone())
        .get_by_id(user_id)
        .await?
        .ok_or(AppError::NotFound("user not found"))?;

    let request = JoinRequest::pending(user_id, user.username, user.display_name);
    JoinRequestRepo::new(state.redis.clone())
        .upsert(lobby_id, &request)
        .await?;

    realtime::publish_lobby_state(&state, lobby_id, "join_request.created").await;

    Ok(Json(request))
}

async fn approve_join_request(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((lobby_id, user_id)): Path<(Uuid, Uuid)>,
) -> AppResult<Json<JoinRequest>> {
    let lobby_id = LobbyId::from(lobby_id);
    let target = UserId::from(user_id);
    let lobby = PgLobbyRepo::new(state.db.clone())
        .get_by_id(lobby_id)
        .await?
        .ok_or(AppError::NotFound("lobby not found"))?;

    if lobby.creator_id != auth.user_id {
        return Err(AppError::Unauthorized("only creator can approve"));
    }
    if !lobby.is_private {
        return Err(AppError::BadRequest(
            "join requests are only for private lobbies".into(),
        ));
    }
    if lobby.status != LobbyStatus::Waiting {
        return Err(AppError::Conflict("lobby is not accepting requests".into()));
    }

    let jr_repo = JoinRequestRepo::new(state.redis.clone());
    let request = jr_repo
        .set_state(lobby_id, target, JoinRequestState::Accepted)
        .await?
        .ok_or(AppError::NotFound("join request not found"))?;

    realtime::publish_lobby_state(&state, lobby_id, "join_request.approved").await;

    Ok(Json(request))
}

async fn reject_join_request(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((lobby_id, user_id)): Path<(Uuid, Uuid)>,
) -> AppResult<Json<JoinRequest>> {
    let lobby_id = LobbyId::from(lobby_id);
    let target = UserId::from(user_id);
    let lobby = PgLobbyRepo::new(state.db.clone())
        .get_by_id(lobby_id)
        .await?
        .ok_or(AppError::NotFound("lobby not found"))?;

    if lobby.creator_id != auth.user_id {
        return Err(AppError::Unauthorized("only creator can reject"));
    }
    if !lobby.is_private {
        return Err(AppError::BadRequest(
            "join requests are only for private lobbies".into(),
        ));
    }
    if lobby.status != LobbyStatus::Waiting {
        return Err(AppError::Conflict("lobby is not accepting requests".into()));
    }

    let jr_repo = JoinRequestRepo::new(state.redis.clone());
    let request = jr_repo
        .set_state(lobby_id, target, JoinRequestState::Rejected)
        .await?
        .ok_or(AppError::NotFound("join request not found"))?;

    realtime::publish_lobby_state(&state, lobby_id, "join_request.rejected").await;

    Ok(Json(request))
}

async fn leave_lobby(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(lobby_id): Path<Uuid>,
    Json(body): Json<UserActionBody>,
) -> AppResult<Json<LobbyResponse>> {
    let lobby_id = LobbyId::from(lobby_id);
    let user_id = auth.user_id;
    let lobbies = PgLobbyRepo::new(state.db.clone());
    let lobby = lobbies
        .get_by_id(lobby_id)
        .await?
        .ok_or(AppError::NotFound("lobby not found"))?;

    if lobby.status != LobbyStatus::Waiting {
        return Err(AppError::Conflict("cannot leave after start".into()));
    }
    if !lobby.participants.iter().any(|p| *p == user_id) {
        return Err(AppError::NotFound("not in lobby"));
    }
    if lobby.creator_id == user_id && lobby.participants.len() > 1 {
        return Err(AppError::BadRequest(
            "host must kick all other players before leaving".into(),
        ));
    }

    let entry = lobby.entry_amount_micro;
    let needs_vault = entry > 0;
    let paid = if !needs_vault {
        0
    } else if lobby.is_sponsored && lobby.creator_id != user_id {
        0
    } else {
        entry
    };
    let vault_txid = require_vault_txid(body.vault_txid.as_deref(), needs_vault)?;
    if needs_vault {
        let addr = custodial_address(&state, user_id).await?;
        let hiro = hiro_client(&state);
        let reader = vault_reader(&state, &hiro);
        reader
            .assert_not_joined(&lobby.path, &addr, vault_txid.as_deref().unwrap())
            .await?;
        refresh_user_balance(&state, user_id).await;
    }

    // Host alone → refund already on-chain; tear down the lobby entirely.
    let host_closing = lobby.creator_id == user_id && lobby.participants.len() == 1;
    if host_closing {
        let mut closed =
            crate::services::lobby_ttl::expire_lobby(&state, lobby_id).await?;
        closed.participants.clear();
        return Ok(Json(LobbyResponse {
            lobby: closed,
            state: None,
            players: vec![],
        }));
    }

    lobbies
        .remove_participant(lobby_id, user_id, paid)
        .await?;
    PlayerStateRepo::new(state.redis.clone())
        .delete(lobby_id, user_id)
        .await
        .ok();
    let _ = SeatHoldRepo::new(state.redis.clone())
        .release(lobby_id, user_id)
        .await;

    let lobby = lobbies
        .get_by_id(lobby_id)
        .await?
        .ok_or(AppError::NotFound("lobby not found"))?;

    if let Some(mut lobby_state) = LobbyStateRepo::new(state.redis.clone())
        .get(lobby_id)
        .await?
    {
        lobby_state.participant_count = lobby.participants.len();
        LobbyStateRepo::new(state.redis.clone())
            .set(&lobby_state)
            .await?;
    }

    announce_lobby_change(
        &state,
        &lobby,
        "player.left",
        json!({ "type": "playerLeft", "userId": user_id }),
    )
    .await;

    Ok(Json(lobby_response(&state, lobby).await?))
}

async fn allocate_path(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> AppResult<Json<serde_json::Value>> {
    let lobbies = PgLobbyRepo::new(state.db.clone());
    let path = generate_unique_lobby_path(&lobbies).await?;
    Ok(Json(serde_json::json!({ "path": path })))
}

/// The creator needs a participant's vault principal to sign the refund that
/// accompanies a kick. Scoped to the host of a waiting lobby.
async fn get_kick_target_address(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((lobby_id, user_id)): Path<(Uuid, Uuid)>,
) -> AppResult<Json<serde_json::Value>> {
    let lobby_id = LobbyId::from(lobby_id);
    let target = UserId::from(user_id);
    let lobby = PgLobbyRepo::new(state.db.clone())
        .get_by_id(lobby_id)
        .await?
        .ok_or(AppError::NotFound("lobby not found"))?;

    if lobby.creator_id != auth.user_id {
        return Err(AppError::Unauthorized("only creator can kick"));
    }
    if !lobby.participants.iter().any(|p| *p == target) {
        return Err(AppError::NotFound("target not in lobby"));
    }

    let address = custodial_address(&state, target).await?;
    Ok(Json(serde_json::json!({ "stxAddress": address })))
}

async fn kick_lobby_player(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(lobby_id): Path<Uuid>,
    Json(body): Json<KickBody>,
) -> AppResult<Json<LobbyResponse>> {
    let lobby_id = LobbyId::from(lobby_id);
    let actor = auth.user_id;
    let target = UserId::from(body.target_user_id);
    let lobbies = PgLobbyRepo::new(state.db.clone());
    let lobby = lobbies
        .get_by_id(lobby_id)
        .await?
        .ok_or(AppError::NotFound("lobby not found"))?;

    if lobby.status != LobbyStatus::Waiting {
        return Err(AppError::Conflict("cannot kick after start".into()));
    }
    if lobby.creator_id != actor {
        return Err(AppError::Unauthorized("only creator can kick"));
    }
    if target == actor {
        return Err(AppError::BadRequest(
            "use leave instead of kicking yourself".into(),
        ));
    }
    if !lobby.participants.iter().any(|p| *p == target) {
        return Err(AppError::NotFound("target not in lobby"));
    }

    let entry = lobby.entry_amount_micro;
    let needs_vault = entry > 0;
    let paid = if !needs_vault {
        0
    } else if lobby.is_sponsored {
        0
    } else {
        entry
    };
    let vault_txid = require_vault_txid(body.vault_txid.as_deref(), needs_vault)?;
    if needs_vault {
        let addr = custodial_address(&state, target).await?;
        let hiro = hiro_client(&state);
        let reader = vault_reader(&state, &hiro);
        reader
            .assert_not_joined(&lobby.path, &addr, vault_txid.as_deref().unwrap())
            .await?;
        refresh_user_balance(&state, target).await;
    }

    lobbies
        .remove_participant(lobby_id, target, paid)
        .await?;
    PlayerStateRepo::new(state.redis.clone())
        .delete(lobby_id, target)
        .await
        .ok();

    let lobby = lobbies
        .get_by_id(lobby_id)
        .await?
        .ok_or(AppError::NotFound("lobby not found"))?;

    if let Some(mut lobby_state) = LobbyStateRepo::new(state.redis.clone())
        .get(lobby_id)
        .await?
    {
        lobby_state.participant_count = lobby.participants.len();
        LobbyStateRepo::new(state.redis.clone())
            .set(&lobby_state)
            .await?;
    }

    announce_lobby_change(
        &state,
        &lobby,
        "player.kicked",
        json!({ "type": "playerKicked", "userId": target, "byUserId": actor }),
    )
    .await;

    Ok(Json(lobby_response(&state, lobby).await?))
}

async fn confirm_vault_claim(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(lobby_id): Path<Uuid>,
    Json(body): Json<VaultClaimBody>,
) -> AppResult<Json<serde_json::Value>> {
    let lobby_id = LobbyId::from(lobby_id);
    let txid = body.vault_txid.trim();
    if txid.is_empty() {
        return Err(AppError::BadRequest("vaultTxid required".into()));
    }
    if body.amount_micro <= 0 {
        return Err(AppError::BadRequest(
            "amountMicro must be positive".into(),
        ));
    }
    let _lobby = PgLobbyRepo::new(state.db.clone())
        .get_by_id(lobby_id)
        .await?
        .ok_or(AppError::NotFound("lobby not found"))?;

    let hiro = hiro_client(&state);
    let reader = vault_reader(&state, &hiro);
    reader.assert_claim_tx(txid).await?;

    let user_id = auth.user_id;
    let svc = WalletChainService::new(state.db.clone(), state.redis.clone(), hiro);
    let bal = svc.refresh_balance(user_id).await?;
    let _ = crate::data::lobby_finished::LobbyFinishedRepo::new(state.redis.clone())
        .mark_claimed(lobby_id)
        .await;
    realtime::publish_wallet_balance(
        &state,
        user_id,
        json!({
            "availableMicro": bal.available_micro,
            "stxAddress": bal.stx_address,
            "payoutMicro": body.amount_micro,
        }),
    );

    Ok(Json(serde_json::json!({
        "ok": true,
        "balance": bal,
    })))
}

async fn set_ready(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(lobby_id): Path<Uuid>,
    Json(body): Json<UserActionBody>,
) -> AppResult<Json<LobbyResponse>> {
    let lobby_id = LobbyId::from(lobby_id);
    let user_id = auth.user_id;
    let lobby = PgLobbyRepo::new(state.db.clone())
        .get_by_id(lobby_id)
        .await?
        .ok_or(AppError::NotFound("lobby not found"))?;
    if lobby.status != LobbyStatus::Waiting {
        return Err(AppError::Conflict("lobby not waiting".into()));
    }

    let ready = body.ready.unwrap_or(true);
    let players = PlayerStateRepo::new(state.redis.clone());
    let mut list = players.list(lobby_id).await?;
    let Some(player) = list.iter_mut().find(|p| p.user_id == user_id) else {
        return Err(AppError::NotFound("not in lobby"));
    };
    player.ready = ready;
    player.updated_at = Utc::now().timestamp();
    players.set(lobby_id, player).await?;

    realtime::publish_lobby_state(&state, lobby_id, "player.ready").await;
    realtime::publish_room_notice(
        &state,
        lobby_id,
        json!({ "type": "playerReady", "userId": user_id, "ready": ready }),
    );

    Ok(Json(lobby_response(&state, lobby).await?))
}

async fn start_lobby(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(lobby_id): Path<Uuid>,
    Json(_body): Json<UserActionBody>,
) -> AppResult<Json<LobbyResponse>> {
    let lobby_id = LobbyId::from(lobby_id);
    let user_id = auth.user_id;
    let lobbies = PgLobbyRepo::new(state.db.clone());
    let lobby = lobbies
        .get_by_id(lobby_id)
        .await?
        .ok_or(AppError::NotFound("lobby not found"))?;

    if lobby.creator_id != user_id {
        return Err(AppError::Unauthorized("only creator can start"));
    }
    if lobby.status != LobbyStatus::Waiting {
        return Err(AppError::Conflict("lobby already started".into()));
    }

    let factory = state
        .games
        .get(&lobby.game_id)
        .ok_or(AppError::NotFound("game not registered"))?;
    let meta = factory.metadata();
    if (lobby.participants.len() as u8) < meta.min_players {
        return Err(AppError::BadRequest(format!(
            "need at least {} players",
            meta.min_players
        )));
    }

    let players = PlayerStateRepo::new(state.redis.clone())
        .list(lobby_id)
        .await?;
    if !players.iter().all(|p| p.ready || p.is_creator) {
        let others_ready = players
            .iter()
            .filter(|p| !p.is_creator)
            .all(|p| p.ready);
        if !others_ready {
            return Err(AppError::BadRequest(
                "all players must be ready".into(),
            ));
        }
    }

    if state.engines.is_running(lobby_id) {
        return Err(AppError::Conflict("match already running".into()));
    }

    lobbies
        .set_status(lobby_id, LobbyStatus::Starting)
        .await?;
    realtime::publish_lobby_state(&state, lobby_id, "lobby.starting").await;

    let host = ServerGameHost::arc(
        lobby_id,
        lobby.path.clone(),
        state.db.clone(),
        lobby.game_id.clone(),
        lobby.entry_amount_micro,
        lobby.pot_micro,
        lobby.creator_id,
        meta.dev_id,
        meta.fee.percentage(),
        state.config.platform_wallet().to_owned(),
        state.redis.clone(),
        state.subscriptions.clone(),
        state.sessions.clone(),
        state.games.clone(),
        state.telegram.clone(),
    );

    let ctx = EngineContext {
        lobby_id,
        game_id: lobby.game_id.clone(),
        player_ids: lobby.participants.clone(),
        creator_id: lobby.creator_id,
        entry_amount_micro: lobby.entry_amount_micro,
        pot_micro: lobby.pot_micro,
        is_sponsored: lobby.is_sponsored,
        settings: serde_json::json!({}),
    };

    let engine = factory
        .create(ctx)
        .map_err(|e| AppError::Internal(anyhow::anyhow!(e.to_string())))?;

    lobbies
        .set_status(lobby_id, LobbyStatus::InProgress)
        .await?;

    let mut lobby_state = LobbyStateRepo::new(state.redis.clone())
        .get(lobby_id)
        .await?
        .unwrap_or_else(|| LobbyState::new(lobby_id, lobby.participants.len()));
    lobby_state.status = LobbyStatus::InProgress;
    lobby_state.started_at = Some(Utc::now().timestamp());
    LobbyStateRepo::new(state.redis.clone())
        .set(&lobby_state)
        .await?;

    let host_ref = host.clone() as sw_plugin::GameHostRef;
    if !state.engines.spawn(lobby_id, engine, host_ref) {
        return Err(AppError::Conflict("match already running".into()));
    }

    let lobby = lobbies
        .get_by_id(lobby_id)
        .await?
        .ok_or(AppError::NotFound("lobby not found"))?;

    announce_lobby_change(
        &state,
        &lobby,
        "lobby.started",
        json!({ "type": "gameStarted", "gameId": lobby.game_id }),
    )
    .await;

    Ok(Json(lobby_response(&state, lobby).await?))
}
