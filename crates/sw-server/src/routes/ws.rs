use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use tracing::{debug, info};

use crate::auth::decode_token;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct WsQuery {
    /// Optional JWT for the socket handshake (shell accepts missing token).
    pub token: Option<String>,
    pub lobby_id: Option<String>,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/ws", get(ws_upgrade))
}

async fn ws_upgrade(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    Query(query): Query<WsQuery>,
) -> impl IntoResponse {
    let authed = query
        .token
        .as_deref()
        .and_then(|token| decode_token(&state.config.jwt_secret, token).ok());

    info!(
        lobby_id = ?query.lobby_id,
        authenticated = authed.is_some(),
        "websocket upgrade"
    );

    ws.on_upgrade(move |socket| handle_socket(socket, query.lobby_id))
}

/// Skeleton socket loop: echoes ping-style text and ignores gameplay.
async fn handle_socket(socket: WebSocket, lobby_id: Option<String>) {
    let (mut sender, mut receiver) = socket.split();

    let _ = sender
        .send(Message::Text(
            serde_json::json!({
                "kind": "system.welcome",
                "payload": {
                    "lobby_id": lobby_id,
                    "message": "Stacks Wars WS shell — gameplay not implemented"
                }
            })
            .to_string()
            .into(),
        ))
        .await;

    while let Some(Ok(msg)) = receiver.next().await {
        match msg {
            Message::Text(text) => {
                let echo = text.to_string();
                debug!(%echo, "ws text (stub)");
                let _ = sender
                    .send(Message::Text(
                        serde_json::json!({
                            "kind": "system.ack",
                            "payload": { "echo": echo }
                        })
                        .to_string()
                        .into(),
                    ))
                    .await;
            }
            Message::Ping(payload) => {
                let _ = sender.send(Message::Pong(payload)).await;
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
}
