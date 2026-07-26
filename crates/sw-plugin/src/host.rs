use async_trait::async_trait;
use sw_domain::UserId;

use crate::{GameMessage, MatchResult, PluginResult};

/// Platform surface exposed to game engines.
///
/// Engines must call only this trait — never server modules, DB pools, or
/// Redis clients. The server provides a concrete adapter at runtime.
#[async_trait]
pub trait GameHost: Send + Sync {
    /// Broadcast a message to every connection in the lobby room.
    async fn broadcast(&self, message: GameMessage) -> PluginResult<()>;

    /// Send a message to a single player in the lobby.
    async fn send_to(&self, user_id: UserId, message: GameMessage) -> PluginResult<()>;

    /// Persist an opaque engine checkpoint (stubbed by the shell host).
    async fn save_checkpoint(&self, state: serde_json::Value) -> PluginResult<()>;

    /// Finish the lobby and hand results back to the platform.
    async fn complete_match(&self, result: MatchResult) -> PluginResult<()>;
}
