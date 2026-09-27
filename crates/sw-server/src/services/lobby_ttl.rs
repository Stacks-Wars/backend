//! Waiting lobbies older than 24h are expired: seats cleared, row deleted,
//! feed notified. Paid seats must be refunded on-chain first (Next cron).
//!
//! Live lobbies (`starting` / `in_progress`) fail differently: the match actor
//! lives in the process, so a restart leaves the row live forever. Those are
//! voided — seats refunded, row finished with a `voided` payload so the room
//! can explain itself.

use chrono::{Duration, Utc};
use serde::Serialize;
use sw_domain::{ChainId, Lobby, LobbyId, LobbyStatus, UserId};
use tracing::{info, warn};
use uuid::Uuid;

use crate::config::USDCX_ASSET_NAME;
use crate::data::join_requests::JoinRequestRepo;
use crate::data::lobbies::PgLobbyRepo;
use crate::data::lobby_finished::LobbyFinishedRepo;
use crate::data::lobby_runtime::{LobbyStateRepo, PlayerStateRepo};
use crate::data::users::PgUserRepo;
use crate::error::{AppError, AppResult};
use crate::services::hiro::HiroClient;
use crate::services::push;
use crate::services::realtime::{self, LobbyFeedKind};
use crate::services::vault_oracle::seat_paid_micro;
use crate::services::vault_verify::VaultReader;
use crate::services::wallet_chain::WalletChainService;
use crate::state::AppState;
use crate::ws::ServerMessage;

pub const LOBBY_TTL: Duration = Duration::hours(24);

/// A live lobby with no engine and no writes for this long is orphaned. Covers
/// the gap between `in_progress` being written and the actor registering.
pub const LIVE_VOID_GRACE: Duration = Duration::minutes(15);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StaleSeat {
    pub user_id: Uuid,
    pub address: String,
    pub paid_micro: i64,
    pub is_creator: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StaleLobby {
    pub lobby: Lobby,
    pub seats: Vec<StaleSeat>,
}

pub async fn list_stale_waiting(state: &AppState) -> AppResult<Vec<StaleLobby>> {
    let cutoff = Utc::now() - LOBBY_TTL;
    let lobbies = PgLobbyRepo::new(state.db.clone())
        .list_waiting_older_than(cutoff)
        .await?;
    stale_lobbies(state, lobbies).await
}

/// Live lobbies whose match actor is gone. Engine presence is read from this
/// process, so a match that is still running is never returned.
pub async fn list_orphaned_live(state: &AppState) -> AppResult<Vec<StaleLobby>> {
    let cutoff = Utc::now() - LIVE_VOID_GRACE;
    let lobbies = PgLobbyRepo::new(state.db.clone())
        .list_live_older_than(cutoff)
        .await?;
    let orphans: Vec<Lobby> = lobbies
        .into_iter()
        .filter(|lobby| !state.engines.is_running(lobby.id))
        .collect();
    stale_lobbies(state, orphans).await
}

async fn stale_lobbies(state: &AppState, lobbies: Vec<Lobby>) -> AppResult<Vec<StaleLobby>> {
    let users = PgUserRepo::new(state.db.clone());

    let mut out = Vec::with_capacity(lobbies.len());
    for lobby in lobbies {
        let mut seats = Vec::with_capacity(lobby.participants.len());
        for user_id in &lobby.participants {
            let wallet = users
                .get_custodial_wallet(*user_id, lobby.chain.as_str())
                .await?;
            let Some(wallet) = wallet else {
                warn!(%user_id, path = %lobby.path, "stale lobby seat missing custodial wallet");
                continue;
            };
            seats.push(StaleSeat {
                user_id: user_id.as_uuid(),
                address: wallet.address,
                paid_micro: seat_paid_micro(
                    lobby.entry_amount_micro,
                    lobby.is_sponsored,
                    lobby.creator_id,
                    *user_id,
                ),
                is_creator: *user_id == lobby.creator_id,
            });
        }
        out.push(StaleLobby { lobby, seats });
    }
    Ok(out)
}

/// Confirm one seat was refunded (or was free), then drop it from the lobby.
/// The caller owns the status guard; this only enforces the refund proof.
pub async fn clear_seat(
    state: &AppState,
    lobby_id: LobbyId,
    user_id: UserId,
    address: &str,
    vault_txid: Option<&str>,
) -> AppResult<()> {
    let lobbies = PgLobbyRepo::new(state.db.clone());
    let lobby = lobbies
        .get_by_id(lobby_id)
        .await?
        .ok_or(AppError::NotFound("lobby not found"))?;
    if !lobby.participants.contains(&user_id) {
        return Err(AppError::NotFound("seat not in lobby"));
    }

    let address = address.trim();
    let paid = seat_paid_micro(
        lobby.entry_amount_micro,
        lobby.is_sponsored,
        lobby.creator_id,
        user_id,
    );

    // Any vault lobby (entry > 0) must prove the seat left the contract map —
    // a sponsored guest still holds a zero-amount seat there.
    if lobby.entry_amount_micro > 0 {
        let txid = vault_txid
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                AppError::BadRequest("vaultTxid required for vault lobby seat".into())
            })?;
        assert_seat_left(state, &lobby, user_id, address, txid).await?;
    }

    lobbies.remove_participant(lobby_id, user_id, paid).await?;
    PlayerStateRepo::new(state.redis.clone())
        .delete(lobby_id, user_id)
        .await
        .ok();
    Ok(())
}

/// Chain matrix for "this seat is out of the vault". The balance refresh is
/// best-effort — the refund transaction is already confirmed.
async fn assert_seat_left(
    state: &AppState,
    lobby: &Lobby,
    user_id: UserId,
    address: &str,
    txid: &str,
) -> AppResult<()> {
    match lobby.chain {
        ChainId::Solana => {
            crate::services::solana_vault::assert_tx_ok(state, txid).await?;
            let _ = crate::services::solana_chain::get_balance(state, user_id).await;
        }
        ChainId::Arbitrum => {
            crate::services::arbitrum_vault::assert_tx_ok(state, txid).await?;
            let _ = crate::services::arbitrum_chain::get_balance(state, user_id).await;
        }
        ChainId::Botchain => {
            crate::services::botchain_vault::assert_tx_ok(state, txid).await?;
            let _ = crate::services::botchain_chain::get_balance(state, user_id).await;
        }
        ChainId::Stacks => {
            let hiro = HiroClient::new(
                state.config.hiro_api_url.clone(),
                state.config.hiro_api_key.clone(),
                &state.config.usdcx_contract,
                USDCX_ASSET_NAME,
                Some(state.config.sw_vault_contract.clone()),
            );
            VaultReader::new(&hiro, &state.config.sw_vault_contract)
                .assert_not_joined(&lobby.path, address, txid)
                .await?;
            let _ = WalletChainService::new(state.db.clone(), state.redis.clone(), hiro)
                .refresh_balance(user_id)
                .await;
        }
    }
    Ok(())
}

/// After on-chain refunds (if any), wipe Redis + Postgres and notify the feed.
pub async fn expire_lobby(state: &AppState, lobby_id: LobbyId) -> AppResult<Lobby> {
    let lobbies = PgLobbyRepo::new(state.db.clone());
    let lobby = lobbies
        .get_by_id(lobby_id)
        .await?
        .ok_or(AppError::NotFound("lobby not found"))?;

    if lobby.status != LobbyStatus::Waiting {
        return Err(AppError::Conflict("only waiting lobbies can expire".into()));
    }

    let players = PlayerStateRepo::new(state.redis.clone());
    for user_id in &lobby.participants {
        let _ = players.delete(lobby_id, *user_id).await;
    }
    let _ = LobbyStateRepo::new(state.redis.clone())
        .clear(lobby_id)
        .await;
    let _ = JoinRequestRepo::new(state.redis.clone())
        .clear_lobby(lobby_id)
        .await;

    lobbies.delete(lobby_id).await?;

    realtime::publish_lobby_feed(state, LobbyFeedKind::Removed, &lobby);
    realtime::publish_game_activity(state).await;
    state.telegram.notify_lobby_deleted(state, &lobby);
    crate::services::push::spawn_lobby_close(
        state.push.clone(),
        state.db.clone(),
        lobby.creator_id,
        lobby.path.clone(),
        lobby.chain,
        lobby.entry_amount_micro,
    );

    info!(
        lobby_id = %lobby_id,
        path = %lobby.path,
        age_hours = (Utc::now() - lobby.created_at).num_hours(),
        "expired stale waiting lobby"
    );

    Ok(lobby)
}

/// Void a live lobby that lost its match actor: clear the room runtime, mark it
/// finished, and persist a `voided` payload so the room can explain itself.
/// Paid seats must already have been refunded on-chain (Next cron).
pub async fn void_lobby(state: &AppState, lobby_id: LobbyId) -> AppResult<Lobby> {
    let lobbies = PgLobbyRepo::new(state.db.clone());
    let lobby = lobbies
        .get_by_id(lobby_id)
        .await?
        .ok_or(AppError::NotFound("lobby not found"))?;

    if !matches!(lobby.status, LobbyStatus::Starting | LobbyStatus::InProgress) {
        return Err(AppError::Conflict("only a live lobby can be voided".into()));
    }
    if !lobby.participants.is_empty() && lobby.entry_amount_micro > 0 {
        return Err(AppError::Conflict(
            "refund all paid seats before voiding lobby".into(),
        ));
    }
    if state.engines.is_running(lobby_id) {
        return Err(AppError::Conflict("match is still running".into()));
    }

    let players = PlayerStateRepo::new(state.redis.clone());
    for user_id in &lobby.participants {
        let _ = players.delete(lobby_id, *user_id).await;
    }
    let _ = LobbyStateRepo::new(state.redis.clone())
        .clear(lobby_id)
        .await;
    let _ = JoinRequestRepo::new(state.redis.clone())
        .clear_lobby(lobby_id)
        .await;

    lobbies.set_status(lobby_id, LobbyStatus::Finished).await?;
    lobbies.mark_voided(lobby_id).await?;
    let lobby = lobbies.get_by_id(lobby_id).await?.unwrap_or(lobby);

    // No `matchId` and no claims: nothing was played, so there is no match to
    // look up and nothing left for a client to claim.
    let payload = serde_json::json!({
        "lobbyId": lobby.id,
        "lobbyPath": lobby.path,
        "voided": true,
        "winners": [],
        "needsOnChainClaim": false,
        "needsOnChainRefund": false,
        "claims": [],
    });
    if let Err(err) = LobbyFinishedRepo::new(state.redis.clone())
        .set(lobby_id, &payload)
        .await
    {
        warn!(error = %err, path = %lobby.path, "failed to persist voided lobby payload");
    }

    state.subscriptions.publish(
        &state.sessions,
        &realtime::lobby_topic(lobby_id),
        ServerMessage {
            kind: "lobby.finished".into(),
            payload,
        },
    );

    let removed = ServerMessage {
        kind: "lobby.removed".into(),
        payload: serde_json::json!({
            "lobbyId": lobby.id,
            "path": lobby.path,
            "gameId": lobby.game_id,
        }),
    };
    for topic in realtime::lobby_feed_topics_for(lobby.entry_amount_micro, lobby.chain) {
        state
            .subscriptions
            .publish(&state.sessions, &topic, removed.clone());
    }
    realtime::publish_game_activity(state).await;
    push::spawn_users_notice(
        state.push.clone(),
        state.db.clone(),
        lobby.participants.iter().map(|id| id.as_uuid()).collect(),
        "Match voided".into(),
        "The match did not finish. Your entry fee is back in your wallet.".into(),
        format!("/room/{}", lobby.path),
    );

    info!(
        lobby_id = %lobby_id,
        path = %lobby.path,
        "voided live lobby with no running match"
    );

    Ok(lobby)
}

/// Best-effort: expire free stale lobbies without on-chain work.
pub async fn expire_free_stale_lobbies(state: &AppState) -> AppResult<usize> {
    let stale = list_stale_waiting(state).await?;
    let mut expired = 0usize;
    for item in stale {
        if item.lobby.entry_amount_micro > 0 {
            continue;
        }
        match expire_lobby(state, item.lobby.id).await {
            Ok(_) => expired += 1,
            Err(err) => warn!(error = %err, path = %item.lobby.path, "free lobby expire failed"),
        }
    }
    Ok(expired)
}
