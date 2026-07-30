use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sw_domain::{
    GameId, Lobby, LobbyId, LobbyState, LobbyStatus, PlayerState, UserId,
};
use sw_plugin::EngineContext;
use uuid::Uuid;

use crate::config::{MIN_ENTRY_MICRO, USDCX_ASSET_NAME, USDCX_CONTRACT};
use crate::auth::AuthUser;
use crate::data::lobbies::{generate_unique_lobby_path, PgLobbyRepo};
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
        .route("/{lobby_id}/join", post(join_lobby))
        .route("/{lobby_id}/leave", post(leave_lobby))
        .route("/{lobby_id}/kick", post(kick_lobby_player))
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

/// Post-vault / money path: always re-read Hiro and rewrite Redis.
async fn refresh_user_balance(state: &AppState, user_id: UserId) {
    let svc = WalletChainService::new(
        state.db.clone(),
        state.redis.clone(),
        hiro_client(state),
    );
    let _ = svc.refresh_balance(user_id).await;
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

async fn list_lobbies(State(state): State<AppState>) -> AppResult<Json<Vec<Lobby>>> {
    let items = PgLobbyRepo::new(state.db.clone())
        .list_open(50, 0)
        .await?;
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

    Ok(Json(LobbyResponse {
        lobby,
        state: Some(lobby_state),
        players: vec![player],
    }))
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

    let meta = state
        .games
        .get(&lobby.game_id)
        .ok_or(AppError::NotFound("game not registered"))?
        .metadata();
    if lobby.participants.len() as u8 >= meta.max_players {
        return Err(AppError::Conflict("lobby is full".into()));
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
        reader
            .assert_joined(&lobby.path, &addr, paid, vault_txid.as_deref().unwrap())
            .await?;
        if let Ok(pot) = reader.get_pot(&lobby.path, &addr).await {
            lobby.pot_micro = pot;
        }
        refresh_user_balance(&state, user_id).await;
    }

    let pot_delta = if lobby.is_sponsored { 0 } else { entry };
    lobbies
        .add_participant(lobby_id, user_id, pot_delta)
        .await?;

    let player = PlayerState::joiner(user_id, user.username, user.display_name);
    PlayerStateRepo::new(state.redis.clone())
        .set(lobby_id, &player)
        .await?;

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

    Ok(Json(lobby_response(&state, lobby).await?))
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
            "creator must be last to leave".into(),
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

    lobbies
        .remove_participant(lobby_id, user_id, paid)
        .await?;
    PlayerStateRepo::new(state.redis.clone())
        .delete(lobby_id, user_id)
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
    let bal_topic = format!("user:{}", user_id.as_uuid());
    state.subscriptions.publish(
        &state.sessions,
        &bal_topic,
        crate::ws::ServerMessage {
            kind: "wallet.balance.updated".into(),
            payload: serde_json::json!({
                "availableMicro": bal.available_micro,
                "stxAddress": bal.stx_address,
                "payoutMicro": body.amount_micro,
            }),
        },
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
    Json(_body): Json<UserActionBody>,
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

    let players = PlayerStateRepo::new(state.redis.clone());
    let mut list = players.list(lobby_id).await?;
    let Some(player) = list.iter_mut().find(|p| p.user_id == user_id) else {
        return Err(AppError::NotFound("not in lobby"));
    };
    player.ready = true;
    player.updated_at = Utc::now().timestamp();
    players.set(lobby_id, player).await?;

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

    lobbies
        .set_status(lobby_id, LobbyStatus::Starting)
        .await?;

    let host = ServerGameHost::arc(
        lobby_id,
        lobby.path.clone(),
        state.db.clone(),
        lobby.game_id.clone(),
        lobby.entry_amount_micro,
        lobby.pot_micro,
        lobby.creator_id,
        meta.fee.percentage(),
        state.redis.clone(),
        state.subscriptions.clone(),
        state.sessions.clone(),
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

    let mut engine = factory
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
    tokio::spawn(async move {
        if let Err(err) = engine.start(host_ref).await {
            tracing::error!(error = %err, "engine start failed");
        }
    });

    let lobby = lobbies
        .get_by_id(lobby_id)
        .await?
        .ok_or(AppError::NotFound("lobby not found"))?;
    Ok(Json(lobby_response(&state, lobby).await?))
}
