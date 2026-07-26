use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sw_domain::{GameId, LobbyId, UserId};

use crate::{GameHost, GameMessage, PlayerEvent, PluginResult};

/// Inputs provided when constructing an engine for a lobby.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineContext {
    pub lobby_id: LobbyId,
    pub game_id: GameId,
    pub player_ids: Vec<UserId>,
    /// Opaque lobby settings from the platform.
    pub settings: serde_json::Value,
}

/// Per-lobby game runtime hosted by the server.
#[async_trait]
pub trait GameEngine: Send + Sync {
    fn game_id(&self) -> &GameId;

    /// Called once when the lobby transitions into active play.
    async fn start(&mut self, host: &dyn GameHost) -> PluginResult<()>;

    /// Player join / leave / ready style events.
    async fn on_player_event(
        &mut self,
        host: &dyn GameHost,
        event: PlayerEvent,
    ) -> PluginResult<()>;

    /// Client → engine gameplay messages.
    async fn on_client_message(
        &mut self,
        host: &dyn GameHost,
        from: UserId,
        message: GameMessage,
    ) -> PluginResult<()>;

    /// Graceful teardown (cancel, disconnect storm, admin abort, …).
    async fn shutdown(&mut self, host: &dyn GameHost) -> PluginResult<()>;
}
