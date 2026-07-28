//! Domain lobby status for SQLx (mirrors sw_domain::LobbyStatus wire values).

use serde::{Deserialize, Serialize};
use sqlx::Type;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[sqlx(type_name = "lobby_status", rename_all = "snake_case")]
#[serde(rename_all = "camelCase")]
pub enum DbLobbyStatus {
    Waiting,
    Starting,
    InProgress,
    Finished,
}

impl From<sw_domain::LobbyStatus> for DbLobbyStatus {
    fn from(value: sw_domain::LobbyStatus) -> Self {
        match value {
            sw_domain::LobbyStatus::Waiting => Self::Waiting,
            sw_domain::LobbyStatus::Starting => Self::Starting,
            sw_domain::LobbyStatus::InProgress => Self::InProgress,
            sw_domain::LobbyStatus::Finished => Self::Finished,
        }
    }
}

impl From<DbLobbyStatus> for sw_domain::LobbyStatus {
    fn from(value: DbLobbyStatus) -> Self {
        match value {
            DbLobbyStatus::Waiting => Self::Waiting,
            DbLobbyStatus::Starting => Self::Starting,
            DbLobbyStatus::InProgress => Self::InProgress,
            DbLobbyStatus::Finished => Self::Finished,
        }
    }
}
