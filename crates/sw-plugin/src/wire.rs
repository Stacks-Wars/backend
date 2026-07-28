use crate::dto::PlayerStateWire;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum GameRoomBroadcast {
    GameStarted,
    GameStartFailed { reason: String },
    FinalStanding { standings: Vec<PlayerStateWire> },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum UserRoomMessage {
    #[serde(rename_all = "camelCase")]
    GameOver {
        rank: usize,
        prize: Option<f64>,
        wars_point: f64,
    },
}
