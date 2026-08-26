use serde::{Deserialize, Serialize};
use serde_json::Value;
use sw_domain::ChainId;
use uuid::Uuid;

/// Public app-wide topic — every connection is auto-subscribed on connect.
/// Cross-chain events live here: game activity, leaderboard, match ticker.
/// Lobby browser deltas go to [`chain_feed_topic`] / [`ALL_FEED_TOPIC`].
pub const APP_TOPIC: &str = "app";

/// Every lobby list delta (paid + free, every chain). Guests subscribe here
/// so they see the full browser without enumerating `ChainId::ALL`.
pub const ALL_FEED_TOPIC: &str = "app:all";

/// Paid/sponsored lobby list deltas for one settlement chain (`app:stacks`).
pub fn chain_feed_topic(chain: ChainId) -> String {
    format!("app:{}", chain.as_str())
}

pub fn parse_chain_feed_topic(topic: &str) -> Option<ChainId> {
    topic.strip_prefix("app:").and_then(|s| s.parse().ok())
}

/// Soft cap on topics per connection to limit abuse.
pub const MAX_TOPICS_PER_CONNECTION: usize = 32;

#[derive(Debug, Clone, Deserialize)]
pub struct Envelope {
    pub kind: String,
    #[serde(default)]
    pub payload: Value,
}

/// Parsed client control messages for this phase.
#[derive(Debug, Clone)]
pub enum ClientMessage {
    Auth {
        token: String,
    },
    Subscribe {
        topic: String,
    },
    Unsubscribe {
        topic: String,
    },
    Ping,
    GameAction {
        lobby_id: Uuid,
        game_id: String,
        action: Value,
    },
    /// Voluntarily leave a match in progress.
    GameQuit {
        lobby_id: Uuid,
    },
    /// Re-request the full room snapshot (used after a reconnect).
    LobbySync {
        lobby_id: Uuid,
    },
    ChatSend {
        lobby_id: Uuid,
        body: String,
    },
    /// Forward-compatible: ignore without error.
    Unknown {
        kind: String,
    },
}

impl ClientMessage {
    pub fn from_envelope(envelope: Envelope) -> Result<Self, String> {
        match envelope.kind.as_str() {
            "auth" => {
                let token = envelope
                    .payload
                    .get("token")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| "payload.token must be a non-empty string".to_owned())?
                    .to_owned();
                Ok(Self::Auth { token })
            }
            "subscribe" => {
                let topic = payload_topic(&envelope.payload)?;
                Ok(Self::Subscribe { topic })
            }
            "unsubscribe" => {
                let topic = payload_topic(&envelope.payload)?;
                Ok(Self::Unsubscribe { topic })
            }
            "ping" => Ok(Self::Ping),
            "game.action" => {
                let lobby_id = envelope
                    .payload
                    .get("lobbyId")
                    .and_then(|v| v.as_str())
                    .and_then(|s| Uuid::parse_str(s).ok())
                    .ok_or_else(|| "payload.lobbyId must be a uuid".to_owned())?;
                let game_id = envelope
                    .payload
                    .get("gameId")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| "payload.gameId must be a non-empty string".to_owned())?
                    .to_owned();
                let action = envelope
                    .payload
                    .get("action")
                    .cloned()
                    .unwrap_or(Value::Null);
                Ok(Self::GameAction {
                    lobby_id,
                    game_id,
                    action,
                })
            }
            "game.quit" => Ok(Self::GameQuit {
                lobby_id: payload_lobby_id(&envelope.payload)?,
            }),
            "lobby.sync" => Ok(Self::LobbySync {
                lobby_id: payload_lobby_id(&envelope.payload)?,
            }),
            "chat.send" => {
                let lobby_id = payload_lobby_id(&envelope.payload)?;
                let body = envelope
                    .payload
                    .get("body")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "payload.body must be a string".to_owned())?
                    .to_owned();
                Ok(Self::ChatSend { lobby_id, body })
            }
            other => Ok(Self::Unknown {
                kind: other.to_owned(),
            }),
        }
    }
}

fn payload_lobby_id(payload: &Value) -> Result<Uuid, String> {
    payload
        .get("lobbyId")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or_else(|| "payload.lobbyId must be a uuid".to_owned())
}

fn payload_topic(payload: &Value) -> Result<String, String> {
    let topic = payload
        .get("topic")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|topic| !topic.is_empty())
        .ok_or_else(|| "payload.topic must be a non-empty string".to_owned())?;
    Ok(topic.to_owned())
}

#[derive(Debug, Clone, Serialize)]
pub struct ServerMessage {
    pub kind: String,
    pub payload: Value,
}

impl ServerMessage {
    pub fn connected(connection_id: Uuid) -> Self {
        Self {
            kind: "connected".into(),
            payload: serde_json::json!({ "connectionId": connection_id }),
        }
    }

    pub fn authenticated(user_id: Uuid) -> Self {
        Self {
            kind: "authenticated".into(),
            payload: serde_json::json!({ "userId": user_id }),
        }
    }

    pub fn subscribed(topic: impl Into<String>) -> Self {
        Self {
            kind: "subscribed".into(),
            payload: serde_json::json!({ "topic": topic.into() }),
        }
    }

    pub fn unsubscribed(topic: impl Into<String>) -> Self {
        Self {
            kind: "unsubscribed".into(),
            payload: serde_json::json!({ "topic": topic.into() }),
        }
    }

    pub fn pong() -> Self {
        Self {
            kind: "pong".into(),
            payload: serde_json::json!({}),
        }
    }

    pub fn error(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            kind: "error".into(),
            payload: serde_json::json!({
                "code": code.into(),
                "message": message.into(),
            }),
        }
    }
}
