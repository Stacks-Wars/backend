use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sw_domain::{GameId, LobbyId, UserId};

use crate::{GameHostRef, PluginResult};

/// Marker for typed client actions.
pub trait GameAction: DeserializeOwned + Send + Sync + 'static {}

/// Marker for typed engine events.
pub trait GameEvent: Serialize + Send + Sync + 'static {}

/// Inputs provided when constructing an engine for a lobby.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineContext {
    pub lobby_id: LobbyId,
    pub game_id: GameId,
    pub player_ids: Vec<UserId>,
    pub creator_id: UserId,
    /// Entry fee in micro-USDCx (0 = free / unset).
    pub entry_amount_micro: i64,
    /// Pot in micro-USDCx.
    pub pot_micro: i64,
    pub is_sponsored: bool,
    pub settings: Value,
}

/// Per-lobby game runtime. Games own their background loops inside [`Self::start`].
#[async_trait]
pub trait GameEngine: Send + Sync {
    fn game_id(&self) -> &GameId;

    /// Initialize roster/board and spawn any timeout / turn loops.
    async fn start(&mut self, host: GameHostRef) -> PluginResult<()>;

    async fn handle_action(
        &mut self,
        host: GameHostRef,
        user_id: UserId,
        action: Value,
    ) -> PluginResult<()>;

    async fn handle_player_quit(
        &mut self,
        host: GameHostRef,
        user_id: UserId,
    ) -> PluginResult<()>;

    async fn get_game_state(&self, user_id: Option<UserId>) -> PluginResult<Value>;

    fn is_finished(&self) -> bool;

    async fn shutdown(&mut self, host: GameHostRef) -> PluginResult<()>;
}
