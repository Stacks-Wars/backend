//! Realtime fan-out.
//!
//! Every WebSocket message the platform pushes outside of game engine events
//! is built here so the wire shapes stay in one place. Topics:
//!
//! - `app` — cross-chain: per-game activity, leaderboard, match ticker.
//! - `app:{chain}` — lobby list deltas for that chain (free events dual-publish).
//! - `app:all` — every lobby list delta; guests subscribe so they see both chains.
//! - `lobby:{id}` — one room: snapshot, state, presence, chat, game events.
//! - `user:{id}` — private: wallet updates, per-player match results.

use serde::Serialize;
use serde_json::{Value, json};
use sw_domain::{
    ChainId, Lobby, LobbyChatMessage, LobbyId, LobbyState, LobbyStatus, PlayerState, UserId,
};

use crate::data::join_requests::{JoinRequest, JoinRequestRepo};
use uuid::Uuid;

use crate::data::chat::LobbyChatRepo;
use crate::data::lobbies::{GameActivity, PgLobbyRepo};
use crate::data::lobby_payouts::LobbyPayoutRepo;
use crate::data::lobby_runtime::{LobbyStateRepo, PlayerStateRepo};
use crate::error::AppResult;
use crate::state::AppState;
use crate::ws::{ALL_FEED_TOPIC, APP_TOPIC, ConnectionId, ServerMessage, chain_feed_topic};

pub fn lobby_topic(lobby_id: LobbyId) -> String {
    format!("lobby:{}", lobby_id.as_uuid())
}

pub fn user_topic(user_id: UserId) -> String {
    format!("user:{}", user_id.as_uuid())
}

/// Parse the lobby id out of a `lobby:{uuid}` topic string.
pub fn parse_lobby_topic(topic: &str) -> Option<LobbyId> {
    topic
        .strip_prefix("lobby:")
        .and_then(|raw| Uuid::parse_str(raw).ok())
        .map(LobbyId::from)
}

/// Paid/sponsored events stay on one chain. Free lobbies never settle on-chain,
/// so they are published to every chain feed. `app:all` always gets a copy so
/// guests (and a third chain later) do not have to subscribe to each `app:{chain}`.
pub fn lobby_feed_topics_for(entry_amount_micro: i64, chain: ChainId) -> Vec<String> {
    let mut topics: Vec<String> = if entry_amount_micro <= 0 {
        ChainId::ALL.iter().copied().map(chain_feed_topic).collect()
    } else {
        vec![chain_feed_topic(chain)]
    };
    topics.push(ALL_FEED_TOPIC.to_owned());
    topics
}

/// Live game runtime attached to a lobby snapshot.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameSnapshot {
    pub game_id: String,
    /// An engine actor is alive and accepting actions.
    pub running: bool,
    /// Engine view for the requesting user, or `null` when unavailable.
    pub state: Value,
}

/// Everything a client needs to render a room without an HTTP round trip.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LobbySnapshot {
    pub lobby: Lobby,
    pub state: Option<LobbyState>,
    pub players: Vec<PlayerState>,
    pub join_requests: Vec<JoinRequest>,
    /// User ids currently subscribed to the room topic.
    pub presence: Vec<Uuid>,
    pub chat: Vec<LobbyChatMessage>,
    pub game: Option<GameSnapshot>,
    /// Present when the lobby has settled (live event or Redis/PG restore).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished: Option<Value>,
    /// Mid-match place claims already issued (`issue_payout`). Reconnects retry.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pending_payouts: Vec<Value>,
}

/// Which users are currently watching a room.
pub fn presence_for(state: &AppState, lobby_id: LobbyId) -> Vec<Uuid> {
    let connections = state.subscriptions.members(&lobby_topic(lobby_id));
    state.sessions.distinct_user_ids(&connections)
}

pub async fn build_lobby_snapshot(
    state: &AppState,
    lobby: Lobby,
    viewer: Option<UserId>,
) -> AppResult<LobbySnapshot> {
    let lobby_id = lobby.id;
    let lobby_state = LobbyStateRepo::new(state.redis.clone())
        .get(lobby_id)
        .await?;
    let mut players = PlayerStateRepo::new(state.redis.clone())
        .list(lobby_id)
        .await?;
    players.sort_by_key(|p| p.joined_at);
    let chat = LobbyChatRepo::new(state.redis.clone())
        .history(lobby_id)
        .await
        .unwrap_or_default();
    let join_requests = JoinRequestRepo::new(state.redis.clone())
        .list(lobby_id)
        .await
        .unwrap_or_default();

    let game = match state.engines.get(lobby_id) {
        Some(handle) => Some(GameSnapshot {
            game_id: handle.game_id().as_str().to_owned(),
            running: true,
            state: handle.snapshot(viewer).await.unwrap_or(Value::Null),
        }),
        None => None,
    };

    let finished = if lobby.status == LobbyStatus::Finished {
        restore_finished_payload(state, &lobby, &mut players).await?
    } else {
        None
    };
    let pending_payouts = LobbyPayoutRepo::new(state.redis.clone())
        .list(lobby_id)
        .await
        .unwrap_or_default();

    Ok(LobbySnapshot {
        lobby,
        state: lobby_state,
        players,
        join_requests,
        presence: presence_for(state, lobby_id),
        chat,
        game,
        finished,
        pending_payouts,
    })
}

/// Redis payload first; fall back to match history for ranks/prizes.
async fn restore_finished_payload(
    state: &AppState,
    lobby: &Lobby,
    players: &mut [PlayerState],
) -> AppResult<Option<Value>> {
    if let Some(payload) = crate::data::lobby_finished::LobbyFinishedRepo::new(state.redis.clone())
        .get(lobby.id)
        .await?
    {
        // Prefer match-history ranks when available; otherwise apply the
        // standings embedded in the finished payload (live-finish path).
        if let Ok(Some((_, _, _, rows))) = crate::data::matches::PgMatchRepo::new(state.db.clone())
            .get_by_lobby(lobby.id)
            .await
        {
            apply_match_player_outcomes(players, &rows);
        } else {
            apply_finished_standings(players, &payload);
        }
        return Ok(Some(payload));
    }

    let Some((match_id, lobby_path, _pot, rows)) =
        crate::data::matches::PgMatchRepo::new(state.db.clone())
            .get_by_lobby(lobby.id)
            .await?
    else {
        return Ok(None);
    };

    apply_match_player_outcomes(players, &rows);

    let winners: Vec<String> = rows
        .iter()
        .filter(|(_, _, is_winner, _, _)| *is_winner)
        .map(|(id, _, _, _, _)| id.to_string())
        .collect();

    Ok(Some(json!({
        "lobbyId": lobby.id,
        "lobbyPath": lobby_path,
        "matchId": match_id,
        "winners": winners,
        // PG-only restore has no claim intent — display standings only.
        "needsOnChainClaim": false,
        "claims": [],
    })))
}

fn apply_match_player_outcomes(
    players: &mut [PlayerState],
    rows: &[(Uuid, Option<i32>, bool, i64, i64)],
) {
    for (user_id, rank, _is_winner, prize_micro, wars_point) in rows {
        if let Some(player) = players.iter_mut().find(|p| p.user_id.as_uuid() == *user_id) {
            player.rank = rank.map(|r| r as usize);
            player.prize_micro = Some(*prize_micro);
            player.wars_point = Some(*wars_point);
        }
    }
}

fn apply_finished_standings(players: &mut [PlayerState], payload: &Value) {
    let Some(standings) = payload.get("standings").and_then(|v| v.as_array()) else {
        return;
    };
    for row in standings {
        let Some(user_id) = row
            .get("userId")
            .and_then(|v| v.as_str())
            .and_then(|s| Uuid::parse_str(s).ok())
        else {
            continue;
        };
        let Some(player) = players.iter_mut().find(|p| p.user_id.as_uuid() == user_id) else {
            continue;
        };
        if let Some(rank) = row.get("rank").and_then(|v| v.as_u64()) {
            player.rank = Some(rank as usize);
        }
        if let Some(prize) = row.get("prizeMicro").and_then(|v| v.as_i64()) {
            player.prize_micro = Some(prize);
        }
        if let Some(points) = row.get("warsPoint").and_then(|v| v.as_i64()) {
            player.wars_point = Some(points);
        }
    }
}

/// Send a full snapshot to a single connection (used right after `subscribe`).
pub async fn send_lobby_snapshot(
    state: &AppState,
    connection_id: ConnectionId,
    lobby_id: LobbyId,
    viewer: Option<UserId>,
) {
    let Ok(Some(lobby)) = PgLobbyRepo::new(state.db.clone()).get_by_id(lobby_id).await else {
        let _ = state.sessions.send(
            connection_id,
            ServerMessage::error("not_found", "lobby not found"),
        );
        return;
    };

    match build_lobby_snapshot(state, lobby, viewer).await {
        Ok(snapshot) => {
            let payload = serde_json::to_value(&snapshot).unwrap_or_else(|_| json!({}));
            let _ = state.sessions.send(
                connection_id,
                ServerMessage {
                    kind: "lobby.snapshot".into(),
                    payload,
                },
            );
        }
        Err(err) => {
            tracing::warn!(%lobby_id, error = %err, "failed to build lobby snapshot");
        }
    }
}

/// Broadcast the room's authoritative lobby + players to everyone in it.
pub async fn publish_lobby_state(state: &AppState, lobby_id: LobbyId, reason: &str) {
    let Ok(Some(lobby)) = PgLobbyRepo::new(state.db.clone()).get_by_id(lobby_id).await else {
        return;
    };
    let lobby_state = LobbyStateRepo::new(state.redis.clone())
        .get(lobby_id)
        .await
        .ok()
        .flatten();
    let mut players = PlayerStateRepo::new(state.redis.clone())
        .list(lobby_id)
        .await
        .unwrap_or_default();
    players.sort_by_key(|p| p.joined_at);
    let join_requests = JoinRequestRepo::new(state.redis.clone())
        .list(lobby_id)
        .await
        .unwrap_or_default();

    state.subscriptions.publish(
        &state.sessions,
        &lobby_topic(lobby_id),
        ServerMessage {
            kind: "lobby.state".into(),
            payload: json!({
                "reason": reason,
                "lobby": lobby,
                "state": lobby_state,
                "players": players,
                "joinRequests": join_requests,
            }),
        },
    );
}

/// Tell the room who is connected right now.
pub fn publish_presence(state: &AppState, lobby_id: LobbyId) {
    let presence = presence_for(state, lobby_id);
    state.subscriptions.publish(
        &state.sessions,
        &lobby_topic(lobby_id),
        ServerMessage {
            kind: "lobby.presence".into(),
            payload: json!({
                "lobbyId": lobby_id,
                "online": presence,
            }),
        },
    );
}

pub fn publish_chat(state: &AppState, message: &LobbyChatMessage) {
    state.subscriptions.publish(
        &state.sessions,
        &lobby_topic(message.lobby_id),
        ServerMessage {
            kind: "lobby.chat".into(),
            payload: serde_json::to_value(message).unwrap_or_else(|_| json!({})),
        },
    );
}

/// A player-facing room notice (joined, left, kicked, ready, started).
pub fn publish_room_notice(state: &AppState, lobby_id: LobbyId, notice: Value) {
    state.subscriptions.publish(
        &state.sessions,
        &lobby_topic(lobby_id),
        ServerMessage {
            kind: "lobby.notice".into(),
            payload: notice,
        },
    );
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LobbyFeedKind {
    Created,
    Updated,
    Removed,
}

impl LobbyFeedKind {
    fn as_kind(self) -> &'static str {
        match self {
            Self::Created => "lobby.created",
            Self::Updated => "lobby.updated",
            Self::Removed => "lobby.removed",
        }
    }
}

/// Global lobby-browser feed. Private lobbies are listed with a lock badge;
/// joining still requires creator approval.
pub fn publish_lobby_feed(state: &AppState, kind: LobbyFeedKind, lobby: &Lobby) {
    let payload = match kind {
        LobbyFeedKind::Removed => json!({
            "lobbyId": lobby.id,
            "path": lobby.path,
            "gameId": lobby.game_id,
        }),
        _ => json!({ "lobby": lobby }),
    };

    let message = ServerMessage {
        kind: kind.as_kind().into(),
        payload,
    };
    for topic in lobby_feed_topics_for(lobby.entry_amount_micro, lobby.chain) {
        state
            .subscriptions
            .publish(&state.sessions, &topic, message.clone());
    }
}

/// Push a lobby to the global feed, choosing the delta kind from its status.
pub fn publish_lobby_change(state: &AppState, lobby: &Lobby) {
    let kind = if lobby.status == LobbyStatus::Finished {
        LobbyFeedKind::Removed
    } else {
        LobbyFeedKind::Updated
    };
    publish_lobby_feed(state, kind, lobby);
}

/// Live lobby / player counts per game, for the games directory.
pub async fn publish_game_activity(state: &AppState) {
    let Ok(activity) = PgLobbyRepo::new(state.db.clone()).game_activity().await else {
        return;
    };
    broadcast_game_activity(state, &activity);
}

pub fn broadcast_game_activity(state: &AppState, activity: &[GameActivity]) {
    state.subscriptions.publish(
        &state.sessions,
        APP_TOPIC,
        ServerMessage {
            kind: "games.activity".into(),
            payload: json!({ "games": activity }),
        },
    );
}

/// Nudge leaderboard subscribers to refetch after a match settles.
pub fn publish_leaderboard_updated(state: &AppState, season_id: Option<i32>, game_id: &str) {
    state.subscriptions.publish(
        &state.sessions,
        APP_TOPIC,
        ServerMessage {
            kind: "leaderboard.updated".into(),
            payload: json!({
                "seasonId": season_id,
                "gameId": game_id,
            }),
        },
    );
}

pub fn publish_quest_updated(state: &AppState, user_id: UserId) {
    publish_quest_updated_raw(&state.subscriptions, &state.sessions, user_id);
}

pub fn publish_quest_updated_raw(
    subscriptions: &crate::ws::SubscriptionManager,
    sessions: &crate::ws::SessionManager,
    user_id: UserId,
) {
    subscriptions.publish(
        sessions,
        &user_topic(user_id),
        ServerMessage {
            kind: "quest.updated".into(),
            payload: json!({ "userId": user_id.as_uuid().to_string() }),
        },
    );
}

/// A finished match, for the landing page ticker and game activity feeds.
pub fn publish_match_finished(state: &AppState, payload: Value) {
    state.subscriptions.publish(
        &state.sessions,
        APP_TOPIC,
        ServerMessage {
            kind: "match.finished".into(),
            payload,
        },
    );
}

pub fn publish_wallet_balance(state: &AppState, user_id: UserId, payload: Value) {
    state.subscriptions.publish(
        &state.sessions,
        &user_topic(user_id),
        ServerMessage {
            kind: "wallet.balance.updated".into(),
            payload,
        },
    );
}
