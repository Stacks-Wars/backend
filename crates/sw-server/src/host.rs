//! Server-side [`GameHost`] — stats + on-chain claim intents on finish.

use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use redis::aio::ConnectionManager;
use serde_json::Value;
use sqlx::PgPool;
use sw_domain::{usdcx_to_micro, GameId, LobbyId, LobbyStatus, MatchId, UserId};
use sw_plugin::{
    calculate_wars_point, GameHost, MatchResult, PlayerResult, PlayerStateWire, PluginError,
    PluginResult, WarsPointContext,
};
use tracing::{error, info, warn};

use sw_plugin::GameRegistry;

use crate::data::lobbies::PgLobbyRepo;
use crate::data::lobby_runtime::{LobbyStateRepo, PlayerStateRepo};
use crate::data::matches::{MatchPlayerRecord, MatchRecord, PgMatchRepo};
use crate::data::seasons::{PgSeasonRepo, SeasonRepo};
use crate::data::stats::{PgStatsRepo, RecordResultInput};
use crate::data::users::PgUserRepo;
use crate::services::telegram::TelegramNotifier;
use crate::services::vault_oracle::{build_claim_intent, split_pot};
use crate::ws::{SessionManager, SubscriptionManager, APP_TOPIC};

pub struct ServerGameHost {
    pub lobby_id: LobbyId,
    pub lobby_path: String,
    pub db: PgPool,
    pub game_id: GameId,
    pub entry_amount_micro: i64,
    pub pot_micro: i64,
    pub creator_id: UserId,
    /// Game plugin `dev_id` — receives the game-fee leg when a custodial wallet exists.
    pub dev_id: UserId,
    pub fee_percentage: u8,
    /// Clarity `PLATFORM-WALLET` principal (vault deployer). Used as claim
    /// `dev-wallet` placeholder when the game fee is forced to 0.
    pub platform_wallet: String,
    pub redis: ConnectionManager,
    pub subscriptions: Arc<SubscriptionManager>,
    pub sessions: Arc<SessionManager>,
    pub games: Arc<GameRegistry>,
    pub telegram: Arc<TelegramNotifier>,
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
        dev_id: UserId,
        fee_percentage: u8,
        platform_wallet: String,
        redis: ConnectionManager,
        subscriptions: Arc<SubscriptionManager>,
        sessions: Arc<SessionManager>,
        games: Arc<GameRegistry>,
        telegram: Arc<TelegramNotifier>,
    ) -> Self {
        Self {
            lobby_id,
            lobby_path,
            db,
            game_id,
            entry_amount_micro,
            pot_micro,
            creator_id,
            dev_id,
            fee_percentage,
            platform_wallet,
            redis,
            subscriptions,
            sessions,
            games,
            telegram,
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
        dev_id: UserId,
        fee_percentage: u8,
        platform_wallet: String,
        redis: ConnectionManager,
        subscriptions: Arc<SubscriptionManager>,
        sessions: Arc<SessionManager>,
        games: Arc<GameRegistry>,
        telegram: Arc<TelegramNotifier>,
    ) -> Arc<Self> {
        Arc::new(Self::new(
            lobby_id,
            lobby_path,
            db,
            game_id,
            entry_amount_micro,
            pot_micro,
            creator_id,
            dev_id,
            fee_percentage,
            platform_wallet,
            redis,
            subscriptions,
            sessions,
            games,
            telegram,
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

    /// Persist the finished match so profiles can show history. Best effort:
    /// a failure here must not block settlement.
    async fn record_match_history(&self, match_id: MatchId, result: &MatchResult) {
        let players = self.get_player_states().await.unwrap_or_default();
        if players.is_empty() {
            return;
        }

        let season_id = PgSeasonRepo::new(self.db.clone())
            .current()
            .await
            .ok()
            .flatten()
            .map(|s| s.id);

        let started_at = LobbyStateRepo::new(self.redis.clone())
            .get(self.lobby_id)
            .await
            .ok()
            .flatten()
            .and_then(|s| s.started_at)
            .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0));

        let entry_micro = self.entry_amount_micro;
        let rows = players
            .iter()
            .map(|p| {
                let user_id = UserId::from(p.user_id);
                MatchPlayerRecord {
                    user_id,
                    rank: p.rank.map(|r| r as i32),
                    is_winner: result.winners.contains(&user_id),
                    prize_micro: p.prize_micro.unwrap_or(0),
                    entry_micro,
                    wars_point: p.wars_point.unwrap_or(0),
                }
            })
            .collect();

        let record = MatchRecord {
            match_id,
            lobby_id: self.lobby_id,
            lobby_path: self.lobby_path.clone(),
            game_id: self.game_id.as_str().to_owned(),
            season_id,
            pot_micro: self.pot_micro,
            entry_amount_micro: entry_micro,
            started_at,
            players: rows,
        };

        if let Err(err) = PgMatchRepo::new(self.db.clone()).record(&record).await {
            error!(
                lobby_id = %self.lobby_id,
                error = %err,
                "failed to persist match history"
            );
        }
    }

    async fn settle(&self, result: &MatchResult) -> PluginResult<()> {
        {
            let mut guard = self.settled.lock();
            if *guard {
                return Ok(());
            }
            *guard = true;
        }

        let match_id = MatchId::new();
        let pot = self.pot_micro;

        let winner = result
            .winners
            .first()
            .copied()
            .or_else(|| result.rankings.first().copied());

        let winner_principal = match winner {
            Some(w) => self.custodial_address(w).await,
            None => None,
        };

        // Resolve game-fee recipient from plugin `dev_id`. Missing user / no
        // custodial wallet → still allow claim: platform principal + fee 0
        // (Clarity ignores the wallet when fee is 0).
        let (dev_wallet, game_fee_pct) =
            match self.custodial_address(self.dev_id).await {
                Some(addr) => (addr, self.fee_percentage),
                None => {
                    warn!(
                        lobby_id = %self.lobby_id,
                        dev_id = %self.dev_id,
                        "dev custodial wallet missing; claiming with platform wallet and 0% game fee"
                    );
                    (self.platform_wallet.clone(), 0u8)
                }
            };

        let (platform_fee_amount, dev_fee_amount, _) = split_pot(pot, game_fee_pct);

        let intent = match (winner, winner_principal) {
            (Some(w), Some(wp)) if pot > 0 => {
                build_claim_intent(pot, game_fee_pct, w, wp, dev_wallet)
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

        self.record_match_history(match_id, result).await;

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
        // Clients render standings from this list — `lobby.state` is not
        // rebroadcast on finish, so ranks must travel with the event.
        let standings = self.build_standings_payload(result).await;
        let finished_payload = serde_json::json!({
            "lobbyId": self.lobby_id,
            "lobbyPath": self.lobby_path,
            "matchId": match_id,
            "winners": result.winners,
            "needsOnChainClaim": intent.is_some(),
            "claims": claims,
            "standings": standings,
        });
        if let Err(err) = crate::data::lobby_finished::LobbyFinishedRepo::new(
            self.redis.clone(),
        )
        .set(self.lobby_id, &finished_payload)
        .await
        {
            error!(
                lobby_id = %self.lobby_id,
                error = %err,
                "failed to persist lobby.finished payload"
            );
        }
        let msg = crate::ws::ServerMessage {
            kind: "lobby.finished".into(),
            payload: finished_payload,
        };
        self.subscriptions.publish(&self.sessions, &topic, msg);

        // Global feed: drop the lobby from the browser, refresh leaderboards,
        // and give the landing page a result to show.
        self.subscriptions.publish(
            &self.sessions,
            APP_TOPIC,
            crate::ws::ServerMessage {
                kind: "lobby.removed".into(),
                payload: serde_json::json!({
                    "lobbyId": self.lobby_id,
                    "path": self.lobby_path,
                    "gameId": self.game_id,
                }),
            },
        );
        self.subscriptions.publish(
            &self.sessions,
            APP_TOPIC,
            crate::ws::ServerMessage {
                kind: "match.finished".into(),
                payload: serde_json::json!({
                    "matchId": match_id,
                    "lobbyId": self.lobby_id,
                    "lobbyPath": self.lobby_path,
                    "gameId": self.game_id,
                    "potMicro": pot,
                    "winners": result.winners,
                }),
            },
        );
        self.subscriptions.publish(
            &self.sessions,
            APP_TOPIC,
            crate::ws::ServerMessage {
                kind: "leaderboard.updated".into(),
                payload: serde_json::json!({ "gameId": self.game_id }),
            },
        );

        self.telegram.notify_lobby_finished_parts(
            self.db.clone(),
            self.redis.clone(),
            self.games.clone(),
            self.lobby_id,
            result,
            self.pot_micro,
            self.fee_percentage,
        );

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

    /// Ordered final standings for the `lobby.finished` payload.
    /// Prefers Redis player ranks (set by `save_player_result`); falls back to
    /// `MatchResult.rankings` order when ranks are missing.
    async fn build_standings_payload(&self, result: &MatchResult) -> Vec<Value> {
        let players = self.get_player_states().await.unwrap_or_default();

        let mut with_rank: Vec<&PlayerStateWire> =
            players.iter().filter(|p| p.rank.is_some()).collect();
        with_rank.sort_by_key(|p| p.rank.unwrap_or(usize::MAX));

        if !with_rank.is_empty() {
            return with_rank
                .into_iter()
                .map(|p| {
                    serde_json::json!({
                        "userId": p.user_id,
                        "rank": p.rank,
                        "prizeMicro": p.prize_micro,
                        "warsPoint": p.wars_point,
                    })
                })
                .collect();
        }

        result
            .rankings
            .iter()
            .enumerate()
            .map(|(idx, user_id)| {
                let player = players
                    .iter()
                    .find(|p| UserId::from(p.user_id) == *user_id);
                serde_json::json!({
                    "userId": user_id,
                    "rank": idx + 1,
                    "prizeMicro": player.and_then(|p| p.prize_micro),
                    "warsPoint": player.and_then(|p| p.wars_point),
                })
            })
            .collect()
    }
}

#[async_trait]
impl GameHost for ServerGameHost {
    /// Engine events are wrapped with routing metadata so a client watching
    /// several topics can dispatch them to the right game component.
    async fn broadcast(&self, payload: Value) -> PluginResult<()> {
        let topic = format!("lobby:{}", self.lobby_id);
        self.subscriptions.publish(
            &self.sessions,
            &topic,
            crate::ws::ServerMessage {
                kind: "lobby.event".into(),
                payload: serde_json::json!({
                    "lobbyId": self.lobby_id,
                    "gameId": self.game_id,
                    "event": payload,
                }),
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
                payload: serde_json::json!({
                    "lobbyId": self.lobby_id,
                    "gameId": self.game_id,
                    "event": payload,
                }),
            },
        );
        Ok(())
    }

    async fn send_except(&self, except_user_id: UserId, payload: Value) -> PluginResult<()> {
        let topic = format!("lobby:{}", self.lobby_id);
        self.subscriptions.publish_except(
            &self.sessions,
            &topic,
            except_user_id,
            crate::ws::ServerMessage {
                kind: "lobby.event".into(),
                payload: serde_json::json!({
                    "lobbyId": self.lobby_id,
                    "gameId": self.game_id,
                    "event": payload,
                }),
            },
        );
        Ok(())
    }

    async fn complete_match(&self, result: MatchResult) -> PluginResult<()> {
        self.settle(&result).await
    }

    async fn finish_lobby(&self) -> PluginResult<()> {
        let players = self.get_player_states().await.unwrap_or_default();
        let mut ranked: Vec<(usize, UserId)> = players
            .iter()
            .filter_map(|p| p.rank.map(|r| (r, UserId::from(p.user_id))))
            .collect();
        ranked.sort_by_key(|(rank, _)| *rank);
        let mut rankings: Vec<UserId> = ranked.into_iter().map(|(_, u)| u).collect();
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

        // Mirror the outcome onto the stored player row so match history and
        // the finished snapshot carry rank, prize and points.
        let players = PlayerStateRepo::new(self.redis.clone());
        if let Ok(rows) = players.list(self.lobby_id).await {
            if let Some(mut row) = rows
                .into_iter()
                .find(|row| row.user_id.as_uuid() == ctx.user_id)
            {
                row.rank = Some(ctx.rank);
                row.prize_micro = ctx.prize.map(usdcx_to_micro);
                row.wars_point = Some(wars_point);
                row.updated_at = chrono::Utc::now().timestamp();
                if let Err(err) = players.set(self.lobby_id, &row).await {
                    error!(
                        lobby_id = %self.lobby_id,
                        user_id = %ctx.user_id,
                        error = %err,
                        "failed to store player result"
                    );
                }
            }
        }

        Ok(PlayerResult {
            rank: ctx.rank,
            prize: ctx.prize,
            wars_point,
        })
    }
}
