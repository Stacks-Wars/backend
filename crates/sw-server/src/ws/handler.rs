use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use futures::{SinkExt, StreamExt};
use tracing::{debug, info};
use uuid::Uuid;

use super::protocol::{ClientMessage, Envelope, ServerMessage, APP_TOPIC};
use super::subscription::SubscribeError;
use crate::state::AppState;
use sw_domain::UserId;

pub fn router() -> Router<AppState> {
    Router::new().route("/app", get(upgrade))
}

async fn upgrade(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
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
            if let Err(err) = authorize_subscribe(state, connection_id, &topic) {
                let _ = state.sessions.send(
                    connection_id,
                    ServerMessage::error("unauthorized", err),
                );
                return;
            }
            match state.subscriptions.subscribe(connection_id, &topic) {
                Ok(()) => {
                    let _ = state
                        .sessions
                        .send(connection_id, ServerMessage::subscribed(topic));
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
            state.subscriptions.unsubscribe(connection_id, &topic);
            let _ = state
                .sessions
                .send(connection_id, ServerMessage::unsubscribed(topic));
        }
        ClientMessage::Ping => {
            let _ = state.sessions.send(connection_id, ServerMessage::pong());
        }
        ClientMessage::GameAction {
            lobby_id,
            game_id,
            action,
        } => {
            let Some(user_id) = state.sessions.user_id(connection_id) else {
                let _ = state.sessions.send(
                    connection_id,
                    ServerMessage::error("unauthorized", "authenticate before game.action"),
                );
                return;
            };
            let topic = format!("lobby:{lobby_id}");
            state.subscriptions.publish(
                &state.sessions,
                &topic,
                ServerMessage {
                    kind: "lobby.event".into(),
                    payload: serde_json::json!({
                        "type": "game",
                        "lobbyId": lobby_id,
                        "gameId": game_id,
                        "userId": user_id.as_uuid(),
                        "event": action,
                    }),
                },
            );
        }
        ClientMessage::Unknown { kind } => {
            debug!(%connection_id, %kind, "ignoring unknown ws message kind");
        }
    }
}

fn authorize_subscribe(
    state: &AppState,
    connection_id: Uuid,
    topic: &str,
) -> Result<(), &'static str> {
    if topic == APP_TOPIC || topic.starts_with("lobby:") {
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
    Ok(())
}

fn cleanup(state: &AppState, connection_id: Uuid) {
    state.subscriptions.unsubscribe_all(connection_id);
    state.sessions.remove(connection_id);
}
