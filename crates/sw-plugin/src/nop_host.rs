use async_trait::async_trait;
use serde_json::Value;
use sw_domain::UserId;

use crate::dto::PlayerStateWire;
use crate::kit::{PlayerResult, WarsPointContext, calculate_wars_point_for};
use crate::{GameHost, MatchResult, PluginResult};

/// Placeholder host used between `GameFactory::create` and `GameEngine::start`.
#[derive(Debug, Default)]
pub struct NopHost;

impl NopHost {
    pub fn arc() -> crate::GameHostRef {
        std::sync::Arc::new(Self)
    }
}

#[async_trait]
impl GameHost for NopHost {
    async fn broadcast(&self, _payload: Value) -> PluginResult<()> {
        Ok(())
    }

    async fn send_to(&self, _user_id: UserId, _payload: Value) -> PluginResult<()> {
        Ok(())
    }

    async fn send_except(&self, _except_user_id: UserId, _payload: Value) -> PluginResult<()> {
        Ok(())
    }

    async fn complete_match(&self, _result: MatchResult) -> PluginResult<()> {
        Ok(())
    }

    async fn finish_lobby(&self) -> PluginResult<()> {
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
        Ok(PlayerResult {
            rank: ctx.rank,
            prize: ctx.prize,
            wars_point: calculate_wars_point_for(ctx, is_winner),
        })
    }
}
