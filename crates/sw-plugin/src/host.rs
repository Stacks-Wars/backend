use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use sw_domain::UserId;

use crate::dto::PlayerStateWire;
use crate::kit::{PlayerResult, WarsPointContext};
use crate::wire::{GameRoomBroadcast, UserRoomMessage};
use crate::{MatchResult, PluginResult};

pub type GameHostRef = Arc<dyn GameHost>;

/// Platform surface exposed to game engines.
#[async_trait]
pub trait GameHost: Send + Sync {
    async fn broadcast(&self, payload: Value) -> PluginResult<()>;

    async fn send_to(&self, user_id: UserId, payload: Value) -> PluginResult<()>;

    async fn send_except(&self, except_user_id: UserId, payload: Value) -> PluginResult<()>;

    async fn complete_match(&self, result: MatchResult) -> PluginResult<()>;

    async fn finish_lobby(&self) -> PluginResult<()>;

    async fn get_player_states(&self) -> PluginResult<Vec<PlayerStateWire>>;

    async fn save_player_result(
        &self,
        ctx: &WarsPointContext,
        is_winner: bool,
    ) -> PluginResult<PlayerResult>;

    /// Pay this player now from the vault pot. Games that want winner-take-all
    /// at `complete_match` should not call this. Default is a no-op so older
    /// hosts keep compiling.
    async fn issue_payout(&self, _user_id: UserId, _amount_micro: i64) -> PluginResult<()> {
        Ok(())
    }

    async fn broadcast_room_game(&self, msg: &GameRoomBroadcast) -> PluginResult<()> {
        let payload =
            serde_json::to_value(msg).map_err(|e| crate::PluginError::Serialization(e.to_string()))?;
        self.broadcast(payload).await
    }

    async fn broadcast_user_room(&self, user_id: UserId, msg: &UserRoomMessage) -> PluginResult<()> {
        let payload =
            serde_json::to_value(msg).map_err(|e| crate::PluginError::Serialization(e.to_string()))?;
        self.send_to(user_id, payload).await
    }
}
