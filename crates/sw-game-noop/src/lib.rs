//! Trivial no-op game used to prove the plugin registration path.
//!
//! It acknowledges start / messages and does not implement real rules.

use std::sync::Arc;

use async_trait::async_trait;
use sw_domain::{GameCatalogEntry, GameId, UserId};
use sw_plugin::{
    EngineContext, GameEngine, GameFactory, GameHost, GameMessage, PlayerEvent, PluginResult,
};
use tracing::info;

pub const NOOP_GAME_ID: &str = "noop";

#[derive(Debug, Default)]
pub struct NoopGameFactory;

impl NoopGameFactory {
    pub fn arc() -> Arc<dyn GameFactory> {
        Arc::new(Self)
    }
}

impl GameFactory for NoopGameFactory {
    fn game_id(&self) -> GameId {
        GameId::new(NOOP_GAME_ID)
    }

    fn catalog_entry(&self) -> GameCatalogEntry {
        GameCatalogEntry {
            id: self.game_id(),
            display_name: "No-op".to_owned(),
            description: "Example plugin that proves registration works.".to_owned(),
            min_players: 1,
            max_players: 16,
            supports_staking: false,
        }
    }

    fn create(&self, ctx: EngineContext) -> PluginResult<Box<dyn GameEngine>> {
        Ok(Box::new(NoopEngine { game_id: ctx.game_id }))
    }
}

struct NoopEngine {
    game_id: GameId,
}

#[async_trait]
impl GameEngine for NoopEngine {
    fn game_id(&self) -> &GameId {
        &self.game_id
    }

    async fn start(&mut self, host: &dyn GameHost) -> PluginResult<()> {
        info!(game_id = %self.game_id, "noop engine started");
        host.broadcast(GameMessage {
            kind: "noop.started".to_owned(),
            payload: serde_json::json!({}),
        })
        .await
    }

    async fn on_player_event(
        &mut self,
        _host: &dyn GameHost,
        event: PlayerEvent,
    ) -> PluginResult<()> {
        info!(?event, "noop player event");
        Ok(())
    }

    async fn on_client_message(
        &mut self,
        host: &dyn GameHost,
        from: UserId,
        message: GameMessage,
    ) -> PluginResult<()> {
        info!(%from, kind = %message.kind, "noop client message");
        host.send_to(
            from,
            GameMessage {
                kind: "noop.ack".to_owned(),
                payload: serde_json::json!({ "echo": message.kind }),
            },
        )
        .await
    }

    async fn shutdown(&mut self, _host: &dyn GameHost) -> PluginResult<()> {
        info!(game_id = %self.game_id, "noop engine shutdown");
        Ok(())
    }
}
