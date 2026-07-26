use serde::{Deserialize, Serialize};

use crate::GameId;

/// Catalog entry for a game that can be hosted by the platform.
///
/// Concrete rules live in external game crates; this is metadata only.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameCatalogEntry {
    pub id: GameId,
    pub display_name: String,
    pub description: String,
    pub min_players: u8,
    pub max_players: u8,
    pub supports_staking: bool,
}
