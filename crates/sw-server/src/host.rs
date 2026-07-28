//! Server-side [`GameHost`] adapter.
//!
//! Broadcast is still stubbed; `save_player_result` persists season stats.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use sqlx::PgPool;
use sw_domain::{usdcx_to_micro, GameId, LobbyId, SeasonId, UserId};
use sw_plugin::{
    calculate_wars_point, GameHost, MatchResult, PlayerResult, PlayerStateWire, PluginError,
    PluginResult, WarsPointContext,
};
use tracing::{error, info};

use crate::data::seasons::{PgSeasonRepo, SeasonRepo};
use crate::data::stats::{PgStatsRepo, RecordResultInput};

#[derive(Debug, Clone)]
pub struct ServerGameHost {
    pub lobby_id: LobbyId,
    pub db: PgPool,
    pub game_id: GameId,
    pub entry_amount_micro: i64,
}

impl ServerGameHost {
    pub fn new(
        lobby_id: LobbyId,
        db: PgPool,
        game_id: GameId,
        entry_amount_micro: i64,
    ) -> Self {
        Self {
            lobby_id,
            db,
            game_id,
            entry_amount_micro,
        }
    }

    pub fn arc(
        lobby_id: LobbyId,
        db: PgPool,
        game_id: GameId,
        entry_amount_micro: i64,
    ) -> Arc<Self> {
        Arc::new(Self::new(lobby_id, db, game_id, entry_amount_micro))
    }

    pub fn from_entry_dollars(
        lobby_id: LobbyId,
        db: PgPool,
        game_id: GameId,
        entry_amount: Option<f64>,
    ) -> Arc<Self> {
        let entry_amount_micro = entry_amount.map(usdcx_to_micro).unwrap_or(0);
        Self::arc(lobby_id, db, game_id, entry_amount_micro)
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
        let won = is_winner || ctx.rank == 1;

        let game_id = ctx
            .game_id
            .clone()
            .unwrap_or_else(|| self.game_id.clone());

        let seasons = PgSeasonRepo::new(self.db.clone());
        let season = seasons
            .current()
            .await
            .map_err(|e| PluginError::Host(e.to_string()))?
            .ok_or_else(|| PluginError::Host("no active season".into()))?;

        let entry_dollars = ctx.entry_amount.or_else(|| {
            if self.entry_amount_micro == 0 {
                None
            } else {
                Some(self.entry_amount_micro as f64 / 1_000_000.0)
            }
        });

        let stats = PgStatsRepo::new(self.db.clone());
        if let Err(err) = stats
            .record_result(RecordResultInput {
                user_id: UserId::from(ctx.user_id),
                game_id: game_id.clone(),
                season_id: SeasonId(season.id.as_i32()),
                points: wars_point,
                is_winner: won,
                prize_dollars: ctx.prize,
                entry_dollars,
            })
            .await
        {
            error!(
                lobby_id = %self.lobby_id,
                user_id = %ctx.user_id,
                error = %err,
                "host.save_player_result stats upsert failed"
            );
            return Err(PluginError::Host(err.to_string()));
        }

        info!(
            lobby_id = %self.lobby_id,
            user_id = %ctx.user_id,
            game_id = %game_id,
            season_id = %season.id,
            is_winner = won,
            wars_point,
            "host.save_player_result"
        );

        Ok(PlayerResult {
            rank: ctx.rank,
            prize: ctx.prize,
            wars_point,
        })
    }
}
