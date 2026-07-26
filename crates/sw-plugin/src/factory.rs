use sw_domain::{GameCatalogEntry, GameId};

use crate::{EngineContext, GameEngine, PluginResult};

/// Constructs engines for a specific game id.
///
/// External game crates implement this and are registered into the server's
/// [`crate::GameRegistry`] at boot.
pub trait GameFactory: Send + Sync {
    fn game_id(&self) -> GameId;

    fn catalog_entry(&self) -> GameCatalogEntry;

    fn create(&self, ctx: EngineContext) -> PluginResult<Box<dyn GameEngine>>;
}
