//! Server-side [`GameHost`] — stats + on-chain claim intents on finish.

use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use redis::aio::ConnectionManager;
use serde_json::Value;
use sqlx::PgPool;
use sw_domain::{GameId, LobbyId, LobbyStatus, UserId};
use sw_plugin::{
    calculate_wars_point, GameHost, MatchResult, PlayerResult, PlayerStateWire, PluginError,
    PluginResult, WarsPointContext,
};
use tracing::{error, info};
use uuid::Uuid;

use crate::data::lobbies::PgLobbyRepo;
use crate::data::lobby_runtime::{LobbyStateRepo, PlayerStateRepo};
use crate::data::seasons::{PgSeasonRepo, SeasonRepo};
use crate::data::stats::{PgStatsRepo, RecordResultInput};
use crate::data::users::PgUserRepo;
use crate::services::vault_oracle::{build_claim_intent, split_pot};
use crate::ws::{SessionManager, SubscriptionManager};

#[derive(Debug)]
pub struct ServerGameHost {
    pub lobby_id: LobbyId,
    pub lobby_path: String,
    pub db: PgPool,
    pub game_id: GameId,
    pub entry_amount_micro: i64,
    pub pot_micro: i64,
    pub creator_id: UserId,
    pub fee_percentage: u8,
    pub redis: ConnectionManager,
    pub subscriptions: Arc<SubscriptionManager>,
    pub sessions: Arc<SessionManager>,
    settled: Mutex<bool>,
}

impl ServerGameHost {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        lobby_id: LobbyId,
        lobby_path: String,
        db: PgPool,
        game_id: GameId,
        entry_amount_micro: i64,
        pot_micro: i64,
        creator_id: UserId,
        fee_percentage: u8,
        redis: ConnectionManager,
        subscriptions: Arc<SubscriptionManager>,
        sessions: Arc<SessionManager>,
    ) -> Self {
        Self {
            lobby_id,
            lobby_path,
            db,
            game_id,
            entry_amount_micro,
            pot_micro,
            creator_id,
            fee_percentage,
            redis,
            subscriptions,
            sessions,
            settled: Mutex::new(false),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn arc(
        lobby_id: LobbyId,
        lobby_path: String,
        db: PgPool,
        game_id: GameId,
        entry_amount_micro: i64,
        pot_micro: i64,
        creator_id: UserId,
        fee_percentage: u8,
        redis: ConnectionManager,
        subscriptions: Arc<SubscriptionManager>,
        sessions: Arc<SessionManager>,
    ) -> Arc<Self> {
        Arc::new(Self::new(
            lobby_id,
            lobby_path,
            db,
            game_id,
            entry_amount_micro,
            pot_micro,
            creator_id,
            fee_percentage,
            redis,
            subscriptions,
            sessions,
        ))
    }

    async fn custodial_address(&self, user_id: UserId) -> Option<String> {
        PgUserRepo::new(self.db.clone())
            .get_custodial_wallet(user_id)
            .await
            .ok()
            .flatten()
            .map(|w| w.stx_address)
    }

    async fn settle(&self, result: &MatchResult) -> PluginResult<()> {
        {
            let mut guard = self.settled.lock();
            if *guard {
                return Ok(());
            }
            *guard = true;
        }

        let match_id = Uuid::now_v7().to_string();
        let pot = self.pot_micro;
        let (platform_fee_amount, dev_fee_amount, _) =
            split_pot(pot, self.fee_percentage);

        let winner = result
            .winners
            .first()
            .copied()
            .or_else(|| result.rankings.first().copied());

        let winner_principal = match winner {
            Some(w) => self.custodial_address(w).await,
            None => None,
        };
        let dev_principal = self.custodial_address(self.creator_id).await;

        let intent = match (winner, winner_principal, dev_principal) {
            (Some(w), Some(wp), Some(dp)) if pot > 0 => {
                build_claim_intent(pot, self.fee_percentage, w, wp, dp)
            }
            _ => None,
        };

        let _ = PgLobbyRepo::new(self.db.clone())
            .set_status(self.lobby_id, LobbyStatus::Finished)
            .await;

        if let Ok(Some(mut st)) = LobbyStateRepo::new(self.redis.clone())
            .get(self.lobby_id)
            .await
        {
            st.status = LobbyStatus::Finished;
            st.finished_at = Some(chrono::Utc::now().timestamp());
            let _ = LobbyStateRepo::new(self.redis.clone()).set(&st).await;
        }

        let topic = format!("lobby:{}", self.lobby_id);
        let claims = match &intent {
            Some(c) => vec![serde_json::json!({
                "userId": c.user_id.as_uuid().to_string(),
                "principal": c.principal,
                "amountMicro": c.amount_micro,
                "nonce": c.nonce,
                "devWallet": c.dev_wallet,
                "devFee": c.dev_fee,
                "role": "winner",
            })],
            None => Vec::new(),
        };
        let msg = crate::ws::ServerMessage {
            kind: "lobby.finished".into(),
            payload: serde_json::json!({
                "lobbyId": self.lobby_id,
                "lobbyPath": self.lobby_path,
                "matchId": match_id,
                "winners": result.winners,
                "needsOnChainClaim": intent.is_some(),
                "claims": claims,
            }),
        };
        self.subscriptions.publish(&self.sessions, &topic, msg);

        info!(
            lobby_id = %self.lobby_id,
            %match_id,
            pot,
            platform_fee_amount,
            dev_fee_amount,
            claims = intent.is_some() as u8,
            "match settled (on-chain claim intent)"
        );
        Ok(())
    }
}

#[async_trait]
impl GameHost for ServerGameHost {
    async fn broadcast(&self, payload: Value) -> PluginResult<()> {
        let topic = format!("lobby:{}", self.lobby_id);
        self.subscriptions.publish(
            &self.sessions,
            &topic,
            crate::ws::ServerMessage {
                kind: "lobby.event".into(),
                payload,
            },
        );
        Ok(())
    }

    async fn send_to(&self, user_id: UserId, payload: Value) -> PluginResult<()> {
        let topic = format!("user:{}", user_id.as_uuid());
        self.subscriptions.publish(
            &self.sessions,
            &topic,
            crate::ws::ServerMessage {
                kind: "user.event".into(),
                payload,
            },
        );
        Ok(())
    }

    async fn send_except(&self, except_user_id: UserId, payload: Value) -> PluginResult<()> {
        let _ = except_user_id;
        self.broadcast(payload).await
    }

    async fn complete_match(&self, result: MatchResult) -> PluginResult<()> {
        self.settle(&result).await
    }

    async fn finish_lobby(&self) -> PluginResult<()> {
        let players = self.get_player_states().await.unwrap_or_default();
        let mut rankings: Vec<UserId> = players
            .iter()
            .filter_map(|p| p.rank.map(|r| (r, UserId::from(p.user_id))))
            .collect::<Vec<_>>()
            .into_iter()
            .map(|(_, u)| u)
            .collect();
        if rankings.is_empty() {
            rankings = players
                .iter()
                .map(|p| UserId::from(p.user_id))
                .collect();
        }
        let winners = rankings.first().copied().into_iter().collect();
        self.settle(&MatchResult {
            winners,
            rankings,
            stats: serde_json::json!({}),
        })
        .await
    }

    async fn get_player_states(&self) -> PluginResult<Vec<PlayerStateWire>> {
        let players = PlayerStateRepo::new(self.redis.clone())
            .list(self.lobby_id)
            .await
            .map_err(|e| PluginError::Host(e.to_string()))?;
        Ok(players
            .into_iter()
            .map(|p| PlayerStateWire {
                user_id: p.user_id.as_uuid(),
                username: p.username,
                display_name: p.display_name,
                lobby_id: self.lobby_id.as_uuid(),
                status: sw_plugin::PlayerStatus::Joined,
                state: sw_plugin::JoinRequestState::Accepted,
                rank: p.rank,
                prize_micro: p.prize_micro,
                wars_point: p.wars_point,
                last_ping: p.last_ping,
                joined_at: p.joined_at,
                updated_at: p.updated_at,
                is_creator: p.is_creator,
                ready: p.ready,
            })
            .collect())
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
                season_id: season.id,
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
                "save_player_result stats failed"
            );
            return Err(PluginError::Host(err.to_string()));
        }

        Ok(PlayerResult {
            rank: ctx.rank,
            prize: ctx.prize,
            wars_point,
        })
    }
}
