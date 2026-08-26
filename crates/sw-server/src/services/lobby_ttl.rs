//! Waiting lobbies older than 24h are expired: seats cleared, row deleted,
//! feed notified. Paid seats must be refunded on-chain first (Next cron).

use chrono::{Duration, Utc};
use serde::Serialize;
use sw_domain::{Lobby, LobbyId, LobbyStatus};
use tracing::{info, warn};
use uuid::Uuid;

use crate::data::join_requests::JoinRequestRepo;
use crate::data::lobbies::PgLobbyRepo;
use crate::data::lobby_runtime::{LobbyStateRepo, PlayerStateRepo};
use crate::data::users::PgUserRepo;
use crate::error::{AppError, AppResult};
use crate::services::realtime::{self, LobbyFeedKind};
use crate::state::AppState;

pub const LOBBY_TTL: Duration = Duration::hours(24);

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
            let paid_micro = if lobby.entry_amount_micro <= 0 {
                0
            } else if lobby.is_sponsored && *user_id != lobby.creator_id {
                0
            } else {
                lobby.entry_amount_micro
            };
            seats.push(StaleSeat {
                user_id: user_id.as_uuid(),
                address: wallet.address,
                paid_micro,
                is_creator: *user_id == lobby.creator_id,
            });
        }
        out.push(StaleLobby { lobby, seats });
    }
    Ok(out)
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
