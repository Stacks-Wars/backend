use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{GameId, UserId};

/// Platform fee taken from a match pot, as a whole-number percent (0–5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeeConfig {
    percentage: u8,
}

impl FeeConfig {
    pub fn new(value: u8) -> Result<Self, FeeError> {
        if value > 5 {
            return Err(FeeError::TooHigh);
        }
        Ok(Self { percentage: value })
    }

    pub fn percentage(&self) -> u8 {
        self.percentage
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum FeeError {
    #[error("fee percentage must be at most 5")]
    TooHigh,
}

/// Catalog categories a game may advertise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GameCategory {
    WordGames,
    Strategy,
    Competitive,
    Trivia,
    CardGames,
    Puzzle,
    Action,
    Casual,
}

/// In-code metadata for a registered game plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameMetadata {
    pub id: GameId,
    pub name: String,
    pub description: String,
    pub min_players: u8,
    pub max_players: u8,
    pub categories: Vec<GameCategory>,
    pub fee: FeeConfig,
    pub dev_id: UserId,
}
