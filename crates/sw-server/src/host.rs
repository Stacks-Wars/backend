//! Server-side [`GameHost`] adapter.
//!
//! Real broadcast / persistence will land here later. The shell logs calls so
//! engines can be exercised without wiring sockets or storage.

use async_trait::async_trait;
use sw_domain::{LobbyId, UserId};
use sw_plugin::{GameHost, GameMessage, MatchResult, PluginResult};
use tracing::info;

#[derive(Debug, Clone)]
pub struct ShellGameHost {
    pub lobby_id: LobbyId,
}

#[async_trait]
impl GameHost for ShellGameHost {
    async fn broadcast(&self, message: GameMessage) -> PluginResult<()> {
        info!(lobby_id = %self.lobby_id, kind = %message.kind, "host.broadcast (stub)");
        Ok(())
    }

    async fn send_to(&self, user_id: UserId, message: GameMessage) -> PluginResult<()> {
        info!(
            lobby_id = %self.lobby_id,
            %user_id,
            kind = %message.kind,
            "host.send_to (stub)"
        );
        Ok(())
    }

    async fn save_checkpoint(&self, state: serde_json::Value) -> PluginResult<()> {
        info!(
            lobby_id = %self.lobby_id,
            bytes = state.to_string().len(),
            "host.save_checkpoint (stub)"
        );
        Ok(())
    }

    async fn complete_match(&self, result: MatchResult) -> PluginResult<()> {
        info!(
            lobby_id = %self.lobby_id,
            winners = result.winners.len(),
            "host.complete_match (stub)"
        );
        Ok(())
    }
}
