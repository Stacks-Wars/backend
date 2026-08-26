use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::get;
use futures::{SinkExt, StreamExt};
use tracing::{debug, info};
use uuid::Uuid;

use super::protocol::{APP_TOPIC, ClientMessage, Envelope, ServerMessage, parse_chain_feed_topic};
use super::subscription::SubscribeError;
use crate::data::chat::LobbyChatRepo;
use crate::data::lobbies::PgLobbyRepo;
use crate::data::users::PgUserRepo;
use crate::engine::DispatchError;
use crate::middleware::rate_limit::{ClientIp, check_ws_connect, rate_limited_response};
use crate::services::realtime;
use crate::state::AppState;
use sw_domain::{LobbyChatMessage, LobbyId, UserId, sanitize_chat_body};

pub fn router() -> Router<AppState> {
    Router::new().route("/app", get(upgrade))
}

async fn upgrade(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
) -> impl IntoResponse {
    let mut redis = state.redis.clone();
    match check_ws_connect(&mut redis, &ip).await {
        Ok(decision) if !decision.allowed => {
            tracing::warn!(%ip, "ws connect rate limit exceeded");
            return rate_limited_response(&decision).into_response();
        }
        Ok(_) => {}
        Err(err) => {
            tracing::warn!(error = %err, "ws connect rate limit redis error; fail-open");
        }
    }

    ws.on_upgrade(move |socket| handle_socket(socket, state))
        .into_response()
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    let sessions = Arc::clone(&state.sessions);
    let subscriptions = Arc::clone(&state.subscriptions);

    let (connection_id, mut outbound_rx) = sessions.insert(None);
    let (mut sink, mut stream) = socket.split();

    let _ = subscriptions.subscribe(connection_id, APP_TOPIC);
    let _ = sessions.send(connection_id, ServerMessage::connected(connection_id));
    let _ = sessions.send(connection_id, ServerMessage::subscribed(APP_TOPIC));

    info!(
        %connection_id,
        sessions = sessions.len(),
        topics = subscriptions.topic_count(),
        "ws /app connected"
    );

    let writer = tokio::spawn(async move {
        while let Some(message) = outbound_rx.recv().await {
            let Ok(text) = serde_json::to_string(&message) else {
                continue;
            };
            if sink.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
        let _ = sink.close().await;
    });

    while let Some(Ok(msg)) = stream.next().await {
        match msg {
            Message::Text(text) => {
                handle_text(&state, connection_id, text.as_str()).await;
            }
            Message::Ping(_) => {
                let _ = state.sessions.send(connection_id, ServerMessage::pong());
            }
            Message::Close(_) => break,
            Message::Pong(_) | Message::Binary(_) => {}
        }
    }

    cleanup(&state, connection_id);
    writer.abort();
    debug!(
        %connection_id,
        sessions = state.sessions.len(),
        topics = state.subscriptions.topic_count(),
        "ws /app disconnected"
    );
}

async fn handle_text(state: &AppState, connection_id: Uuid, text: &str) {
    let envelope = match serde_json::from_str::<Envelope>(text) {
        Ok(envelope) => envelope,
        Err(err) => {
            let _ = state.sessions.send(
                connection_id,
                ServerMessage::error("bad_request", format!("invalid json: {err}")),
            );
            return;
        }
    };

    let message = match ClientMessage::from_envelope(envelope) {
        Ok(message) => message,
        Err(err) => {
            let _ = state
                .sessions
                .send(connection_id, ServerMessage::error("bad_request", err));
            return;
        }
    };

    match message {
        ClientMessage::Auth { token } => match state.jwt.verify(&token).await {
            Ok(claims) => {
                let user_id = UserId::from(claims.user_id);
                if !state.sessions.bind_user(connection_id, user_id) {
                    return;
                }
                let _ = state
                    .sessions
                    .send(connection_id, ServerMessage::authenticated(claims.user_id));
            }
            Err(err) => {
                let _ = state.sessions.send(
                    connection_id,
                    ServerMessage::error("unauthorized", err.to_string()),
                );
            }
        },
        ClientMessage::Subscribe { topic } => {
            let Some(topic) = resolve_topic(state, connection_id, topic).await else {
                return;
            };
            if let Err(err) = authorize_subscribe(state, connection_id, &topic) {
                let _ = state
                    .sessions
                    .send(connection_id, ServerMessage::error("unauthorized", err));
                return;
            }
            match state.subscriptions.subscribe(connection_id, &topic) {
                Ok(()) => {
                    let _ = state
                        .sessions
                        .send(connection_id, ServerMessage::subscribed(topic.clone()));

                    // A room subscription is the client's only fetch: hand back
                    // the full snapshot, then tell the room someone arrived.
                    if let Some(lobby_id) = realtime::parse_lobby_topic(&topic) {
                        let viewer = state.sessions.user_id(connection_id);
                        realtime::send_lobby_snapshot(state, connection_id, lobby_id, viewer).await;
                        realtime::publish_presence(state, lobby_id);
                    }
                }
                Err(SubscribeError::TopicLimitReached) => {
                    let _ = state.sessions.send(
                        connection_id,
                        ServerMessage::error(
                            "topic_limit",
                            "too many topic subscriptions on this connection",
                        ),
                    );
                }
            }
        }
        ClientMessage::Unsubscribe { topic } => {
            let Some(topic) = resolve_topic(state, connection_id, topic).await else {
                return;
            };
            state.subscriptions.unsubscribe(connection_id, &topic);
            let _ = state
                .sessions
                .send(connection_id, ServerMessage::unsubscribed(topic.clone()));
            if let Some(lobby_id) = realtime::parse_lobby_topic(&topic) {
                realtime::publish_presence(state, lobby_id);
            }
        }
        ClientMessage::Ping => {
            let _ = state.sessions.send(connection_id, ServerMessage::pong());
        }
        ClientMessage::LobbySync { lobby_id } => {
            let viewer = state.sessions.user_id(connection_id);
            realtime::send_lobby_snapshot(state, connection_id, LobbyId::from(lobby_id), viewer)
                .await;
        }
        ClientMessage::GameAction {
            lobby_id,
            game_id,
            action,
        } => {
            let Some(user_id) = require_auth(state, connection_id, "game.action") else {
                return;
            };
            let lobby_id = LobbyId::from(lobby_id);

            let Some(engine) = state.engines.get(lobby_id) else {
                let _ = state.sessions.send(
                    connection_id,
                    ServerMessage::error("no_engine", DispatchError::NotRunning.as_str()),
                );
                return;
            };

            if engine.game_id().as_str() != game_id {
                let _ = state.sessions.send(
                    connection_id,
                    ServerMessage::error("game_mismatch", "gameId does not match this lobby"),
                );
                return;
            }

            if let Err(err) = engine.send_action(user_id, action) {
                let _ = state.sessions.send(
                    connection_id,
                    ServerMessage::error("engine_busy", err.as_str()),
                );
            }
        }
        ClientMessage::GameQuit { lobby_id } => {
            let Some(user_id) = require_auth(state, connection_id, "game.quit") else {
                return;
            };
            if let Some(engine) = state.engines.get(LobbyId::from(lobby_id)) {
                let _ = engine.send_quit(user_id);
            }
        }
        ClientMessage::ChatSend { lobby_id, body } => {
            let Some(user_id) = require_auth(state, connection_id, "chat.send") else {
                return;
            };
            handle_chat(state, connection_id, LobbyId::from(lobby_id), user_id, body).await;
        }
        ClientMessage::Unknown { kind } => {
            debug!(%connection_id, %kind, "ignoring unknown ws message kind");
        }
    }
}

/// Expand the `lobbyPath:{path}` alias into the canonical `lobby:{uuid}` topic
/// so a client holding only a share link can subscribe without an HTTP lookup.
async fn resolve_topic(state: &AppState, connection_id: Uuid, topic: String) -> Option<String> {
    let Some(path) = topic.strip_prefix("lobbyPath:") else {
        return Some(topic);
    };

    match PgLobbyRepo::new(state.db.clone()).get_by_path(path).await {
        Ok(Some(lobby)) => Some(realtime::lobby_topic(lobby.id)),
        _ => {
            let _ = state.sessions.send(
                connection_id,
                ServerMessage::error("not_found", format!("no lobby at /{path}")),
            );
            None
        }
    }
}

fn require_auth(state: &AppState, connection_id: Uuid, action: &str) -> Option<UserId> {
    match state.sessions.user_id(connection_id) {
        Some(user_id) => Some(user_id),
        None => {
            let _ = state.sessions.send(
                connection_id,
                ServerMessage::error("unauthorized", format!("authenticate before {action}")),
            );
            None
        }
    }
}

/// Only lobby participants may post; history is capped in Redis.
async fn handle_chat(
    state: &AppState,
    connection_id: Uuid,
    lobby_id: LobbyId,
    user_id: UserId,
    body: String,
) {
    let Some(body) = sanitize_chat_body(&body) else {
        return;
    };

    let lobby = match PgLobbyRepo::new(state.db.clone()).get_by_id(lobby_id).await {
        Ok(Some(lobby)) => lobby,
        _ => {
            let _ = state.sessions.send(
                connection_id,
                ServerMessage::error("not_found", "lobby not found"),
            );
            return;
        }
    };

    if !lobby.participants.iter().any(|p| *p == user_id) {
        let _ = state.sessions.send(
            connection_id,
            ServerMessage::error("forbidden", "join the lobby to chat"),
        );
        return;
    }

    let user = PgUserRepo::new(state.db.clone())
        .get_by_id(user_id)
        .await
        .ok()
        .flatten();

    let message = LobbyChatMessage::new(
        lobby_id,
        user_id,
        user.as_ref().and_then(|u| u.username.clone()),
        user.as_ref().and_then(|u| u.display_name.clone()),
        body,
    );

    if let Err(err) = LobbyChatRepo::new(state.redis.clone())
        .append(&message)
        .await
    {
        debug!(%lobby_id, error = %err, "failed to persist chat message");
    }
    realtime::publish_chat(state, &message);
}

fn authorize_subscribe(
    state: &AppState,
    connection_id: Uuid,
    topic: &str,
) -> Result<(), &'static str> {
    if topic == APP_TOPIC || parse_chain_feed_topic(topic).is_some() || topic.starts_with("lobby:")
    {
        return Ok(());
    }
    if let Some(rest) = topic.strip_prefix("user:") {
        let Some(bound) = state.sessions.user_id(connection_id) else {
            return Err("authenticate before subscribing to user topics");
        };
        let Ok(wanted) = Uuid::parse_str(rest) else {
            return Err("invalid user topic");
        };
        if bound.as_uuid() != wanted {
            return Err("cannot subscribe to another user's topic");
        }
        return Ok(());
    }
    Err("unknown topic")
}

fn cleanup(state: &AppState, connection_id: Uuid) {
    let topics = state.subscriptions.topics_for(connection_id);
    state.subscriptions.unsubscribe_all(connection_id);
    state.sessions.remove(connection_id);

    for topic in topics {
        if let Some(lobby_id) = realtime::parse_lobby_topic(&topic) {
            realtime::publish_presence(state, lobby_id);
        }
    }
}
