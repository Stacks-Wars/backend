use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// Public app-wide topic — every connection is auto-subscribed on connect.
pub const APP_TOPIC: &str = "app";

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
    Subscribe { topic: String },
    Unsubscribe { topic: String },
    Ping,
    /// Forward-compatible: ignore without error.
    Unknown { kind: String },
}

impl ClientMessage {
    pub fn from_envelope(envelope: Envelope) -> Result<Self, String> {
        match envelope.kind.as_str() {
            "subscribe" => {
                let topic = payload_topic(&envelope.payload)?;
                Ok(Self::Subscribe { topic })
            }
            "unsubscribe" => {
                let topic = payload_topic(&envelope.payload)?;
                Ok(Self::Unsubscribe { topic })
            }
            "ping" => Ok(Self::Ping),
            other => Ok(Self::Unknown {
                kind: other.to_owned(),
            }),
        }
    }
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
