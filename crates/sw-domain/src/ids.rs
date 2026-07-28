use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;
use uuid::Uuid;

macro_rules! id_newtype {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }

            pub fn as_uuid(&self) -> Uuid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl From<Uuid> for $name {
            fn from(value: Uuid) -> Self {
                Self(value)
            }
        }
    };
}

id_newtype!(UserId);
id_newtype!(LobbyId);
id_newtype!(MatchId);

/// Auto-increment season id from Postgres `SERIAL`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SeasonId(pub i32);

impl SeasonId {
    pub fn as_i32(self) -> i32 {
        self.0
    }
}

impl std::fmt::Display for SeasonId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<i32> for SeasonId {
    fn from(value: i32) -> Self {
        Self(value)
    }
}

/// Min / max length for a game slug (URL path + registry key).
pub const GAME_ID_MIN_LEN: usize = 3;
/// Enough for names like `lexi-wars` / `ludo-rush`; keeps path segments tidy.
pub const GAME_ID_MAX_LEN: usize = 32;

/// Stable string identifier for a registered game plugin (e.g. `"checkers"`, `"lexi-wars"`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct GameId(String);

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GameIdError {
    #[error("game id must be between {GAME_ID_MIN_LEN} and {GAME_ID_MAX_LEN} characters")]
    InvalidLength,
}

impl GameId {
    pub fn new(id: impl Into<String>) -> Result<Self, GameIdError> {
        let id = id.into();
        let len = id.chars().count();
        if len < GAME_ID_MIN_LEN || len > GAME_ID_MAX_LEN {
            return Err(GameIdError::InvalidLength);
        }
        Ok(Self(id))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for GameId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl<'de> Deserialize<'de> for GameId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        GameId::new(raw).map_err(serde::de::Error::custom)
    }
}

impl TryFrom<&str> for GameId {
    type Error = GameIdError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<String> for GameId {
    type Error = GameIdError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}
