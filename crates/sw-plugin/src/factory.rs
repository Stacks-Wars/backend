use sw_domain::{GameId, GameMetadata};

use crate::{EngineContext, GameEngine, PluginResult};

/// Constructs engines for a specific game id.
///
/// External game crates implement this and are registered into the server's
/// [`crate::GameRegistry`] at boot.
pub trait GameFactory: Send + Sync {
    fn game_id(&self) -> GameId;

    fn metadata(&self) -> GameMetadata;

    fn create(&self, ctx: EngineContext) -> PluginResult<Box<dyn GameEngine>>;
}
