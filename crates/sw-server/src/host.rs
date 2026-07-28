//! Server-side [`GameHost`] adapter.
//!
//! Real broadcast / persistence will land here later. Calls are logged so
//! engines can be exercised without wiring sockets or storage.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use sw_domain::{LobbyId, UserId};
use sw_plugin::{
    calculate_wars_point, GameHost, MatchResult, PlayerResult, PlayerStateWire, PluginResult,
    WarsPointContext,
};
use tracing::info;

#[derive(Debug, Clone)]
pub struct ServerGameHost {
    pub lobby_id: LobbyId,
}

impl ServerGameHost {
    pub fn arc(lobby_id: LobbyId) -> Arc<Self> {
        Arc::new(Self { lobby_id })
    }
}

#[async_trait]
impl GameHost for ServerGameHost {
    async fn broadcast(&self, payload: Value) -> PluginResult<()> {
        info!(lobby_id = %self.lobby_id, payload = %payload, "host.broadcast (stub)");
        Ok(())
    }

    async fn send_to(&self, user_id: UserId, payload: Value) -> PluginResult<()> {
        info!(
            lobby_id = %self.lobby_id,
            %user_id,
            payload = %payload,
            "host.send_to (stub)"
        );
        Ok(())
    }

    async fn send_except(&self, except_user_id: UserId, payload: Value) -> PluginResult<()> {
        info!(
            lobby_id = %self.lobby_id,
            %except_user_id,
            payload = %payload,
            "host.send_except (stub)"
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

    async fn finish_lobby(&self) -> PluginResult<()> {
        info!(lobby_id = %self.lobby_id, "host.finish_lobby (stub)");
        Ok(())
    }

    async fn get_player_states(&self) -> PluginResult<Vec<PlayerStateWire>> {
        Ok(vec![])
    }

    async fn save_player_result(
        &self,
        ctx: &WarsPointContext,
        is_winner: bool,
    ) -> PluginResult<PlayerResult> {
        let wars_point = calculate_wars_point(ctx);
        info!(
            lobby_id = %self.lobby_id,
            user_id = %ctx.user_id,
            is_winner,
            wars_point,
            "host.save_player_result (stub)"
        );
        Ok(PlayerResult {
            rank: ctx.rank,
            prize: ctx.prize,
            wars_point,
        })
    }
}
