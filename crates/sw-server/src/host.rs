//! Server-side [`GameHost`] — stats + on-chain claim intents on finish.

use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use redis::aio::ConnectionManager;
use serde_json::Value;
use sqlx::PgPool;
use sw_domain::{ChainId, GameId, LobbyId, LobbyStatus, MatchId, UserId, usdcx_to_micro};
use sw_plugin::{
    GameHost, MatchResult, PlayerResult, PlayerStateWire, PluginError, PluginResult,
    WarsPointContext, calculate_wars_point,
};
use tracing::{error, info, warn};
use uuid::Uuid;

use sw_plugin::GameRegistry;

use crate::data::lobbies::PgLobbyRepo;
use crate::data::lobby_payouts::LobbyPayoutRepo;
use crate::data::lobby_runtime::{LobbyStateRepo, PlayerStateRepo};
use crate::data::matches::{MatchPlayerRecord, MatchRecord, PgMatchRepo};
use crate::data::seasons::{PgSeasonRepo, SeasonRepo};
use crate::data::stats::{PgStatsRepo, RecordResultInput};
use crate::data::users::PgUserRepo;
use crate::services::push::PushService;
use crate::services::realtime;
use crate::services::telegram::TelegramNotifier;
use crate::services::vault_oracle::{
    build_claim_intent, draw_refund_claims as build_draw_refunds, is_distributed_settlement,
    is_draw_result, split_pot, winner_for_claim,
};
use crate::ws::{APP_TOPIC, SessionManager, SubscriptionManager};

pub struct ServerGameHost {
    pub lobby_id: LobbyId,
    pub lobby_path: String,
    pub lobby_name: String,
    pub is_private: bool,
    pub db: PgPool,
    pub game_id: GameId,
    pub chain: ChainId,
    pub entry_amount_micro: i64,
    pub pot_micro: i64,
    pub creator_id: UserId,
    /// Game plugin `dev_id` — receives the game-fee leg when a custodial wallet exists.
    pub dev_id: UserId,
    pub fee_percentage: u8,
    /// Claim `dev-wallet` fallback when the game author has no custodial
    /// wallet on this lobby's chain (Stacks principal or Solana wars pubkey).
    pub platform_wallet: String,
    pub redis: ConnectionManager,
    pub subscriptions: Arc<SubscriptionManager>,
    pub sessions: Arc<SessionManager>,
    pub games: Arc<GameRegistry>,
    pub telegram: Arc<TelegramNotifier>,
    pub push: crate::services::push::PushService,
    settled: Mutex<bool>,
}

impl ServerGameHost {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        lobby_id: LobbyId,
        lobby_path: String,
        lobby_name: String,
        is_private: bool,
        db: PgPool,
        game_id: GameId,
        chain: ChainId,
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
        push: PushService,
    ) -> Self {
        Self {
            lobby_id,
            lobby_path,
            lobby_name,
            is_private,
            db,
            game_id,
            chain,
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
            push,
            settled: Mutex::new(false),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn arc(
        lobby_id: LobbyId,
        lobby_path: String,
        lobby_name: String,
        is_private: bool,
        db: PgPool,
        game_id: GameId,
        chain: ChainId,
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
        push: PushService,
    ) -> Arc<Self> {
        Arc::new(Self::new(
            lobby_id,
            lobby_path,
            lobby_name,
            is_private,
            db,
            game_id,
            chain,
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
            push,
        ))
    }

    async fn custodial_address(&self, user_id: UserId) -> Option<String> {
        PgUserRepo::new(self.db.clone())
            .get_custodial_wallet(user_id, self.chain.as_str())
            .await
            .ok()
            .flatten()
            .map(|w| w.address)
    }

    async fn draw_refund_claims(&self) -> Vec<serde_json::Value> {
        let lobby = match PgLobbyRepo::new(self.db.clone())
            .get_by_id(self.lobby_id)
            .await
        {
            Ok(Some(lobby)) => lobby,
            _ => return Vec::new(),
        };
        let mut seats = Vec::new();
        for user_id in lobby.participants.iter().copied() {
            seats.push((user_id, self.custodial_address(user_id).await));
        }
        build_draw_refunds(
            seats,
            lobby.entry_amount_micro,
            lobby.is_sponsored,
            lobby.creator_id,
        )
    }

    /// Missing dest user (prod UUID on dest) → 0% fee. Active user without a
    /// wallet on this chain keeps the configured %; the claim path provisions.
    async fn resolve_dest_fee(&self) -> (String, u8, Option<UserId>, bool) {
        if self.fee_percentage == 0 {
            return (self.platform_wallet.clone(), 0, None, false);
        }
        let repo = PgUserRepo::new(self.db.clone());
        match repo.get_active_by_id(self.dev_id).await {
            Ok(Some(_)) => match self.custodial_address(self.dev_id).await {
                Some(addr) => (addr, self.fee_percentage, Some(self.dev_id), false),
                None => {
                    info!(
                        lobby_id = %self.lobby_id,
                        dest_id = %self.dev_id,
                        chain = %self.chain.as_str(),
                        "dest has no wallet on this chain; claim will provision one"
                    );
                    (
                        self.platform_wallet.clone(),
                        self.fee_percentage,
                        Some(self.dev_id),
                        true,
                    )
                }
            },
            _ => {
                warn!(
                    lobby_id = %self.lobby_id,
                    dest_id = %self.dev_id,
                    "dest user missing; claiming with platform wallet and 0% game fee"
                );
                (self.platform_wallet.clone(), 0, None, false)
            }
        }
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
        let prior_payouts = LobbyPayoutRepo::new(self.redis.clone())
            .list(self.lobby_id)
            .await
            .unwrap_or_default();
        let distributed = is_distributed_settlement(result) || !prior_payouts.is_empty();
        let draw = !distributed && is_draw_result(result);
        let winner = if draw || distributed {
            None
        } else {
            winner_for_claim(result)
        };

        let winner_principal = match winner {
            Some(w) => self.custodial_address(w).await,
            None => None,
        };

        let (dest_wallet, game_fee_pct, dest_id, dest_needs_wallet) = self.resolve_dest_fee().await;

        let (platform_fee_amount, dev_fee_amount, _) = split_pot(pot, game_fee_pct);

        let intent = match (winner, winner_principal) {
            (Some(w), Some(wp)) if pot > 0 => build_claim_intent(
                pot,
                game_fee_pct,
                w,
                wp,
                dest_wallet,
                dest_id,
                dest_needs_wallet,
            ),
            _ => None,
        };

        let refunds = if draw && pot > 0 {
            self.draw_refund_claims().await
        } else {
            Vec::new()
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
        let claims = if !refunds.is_empty() {
            refunds
        } else if !prior_payouts.is_empty() {
            prior_payouts
        } else {
            match &intent {
                Some(c) => vec![serde_json::json!({
                    "userId": c.user_id.as_uuid().to_string(),
                    "principal": c.principal,
                    "amountMicro": c.amount_micro,
                    "nonce": c.nonce,
                    "devWallet": c.dest_wallet,
                    "devFee": c.dest_fee,
                    "devId": c.dest_id.map(|id| id.as_uuid().to_string()),
                    "devNeedsWallet": c.dest_needs_wallet,
                    "role": "winner",
                })],
                None => Vec::new(),
            }
        };
        let needs_refund = claims.iter().any(|c| c.get("role").and_then(|v| v.as_str()) == Some("refund"));
        let needs_claim = !needs_refund
            && claims.iter().any(|c| {
                c.get("amountMicro").and_then(|v| v.as_i64()).unwrap_or(0) > 0
                    && c.get("role").and_then(|v| v.as_str()) != Some("refund")
            });
        // Clients render standings from this list — `lobby.state` is not
        // rebroadcast on finish, so ranks must travel with the event.
        let standings = self.build_standings_payload(result).await;
        let finished_payload = serde_json::json!({
            "lobbyId": self.lobby_id,
            "lobbyPath": self.lobby_path,
            "matchId": match_id,
            "winners": result.winners,
            "needsOnChainClaim": needs_claim,
            "needsOnChainRefund": needs_refund,
            "claims": claims,
            "standings": standings,
        });
        if let Err(err) = crate::data::lobby_finished::LobbyFinishedRepo::new(self.redis.clone())
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

        let winner_ids: Vec<Uuid> = result.winners.iter().map(|w| w.as_uuid()).collect();
        crate::services::push::spawn_users_notice(
            self.push.clone(),
            self.db.clone(),
            winner_ids,
            "Match finished".into(),
            "Standings are up.".into(),
            format!("/room/{}", self.lobby_path),
        );

        // Chain-scoped browser feed: drop the lobby from subscribers who
        // would have seen it, refresh leaderboards, landing ticker stays on `app`.
        let removed = crate::ws::ServerMessage {
            kind: "lobby.removed".into(),
            payload: serde_json::json!({
                "lobbyId": self.lobby_id,
                "path": self.lobby_path,
                "gameId": self.game_id,
            }),
        };
        for topic in realtime::lobby_feed_topics_for(self.entry_amount_micro, self.chain) {
            self.subscriptions
                .publish(&self.sessions, &topic, removed.clone());
        }
        crate::services::push::spawn_lobby_close(
            self.push.clone(),
            self.db.clone(),
            self.creator_id,
            self.lobby_path.clone(),
            self.chain,
            self.entry_amount_micro,
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
                    "lobbyName": self.lobby_name,
                    "gameId": self.game_id,
                    "chain": self.chain.as_str(),
                    "potMicro": pot,
                    "entryAmountMicro": self.entry_amount_micro,
                    "playerCount": result.rankings.len().max(result.winners.len()),
                    "isPrivate": self.is_private,
                    "finishedAt": chrono::Utc::now().to_rfc3339(),
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
                let player = players.iter().find(|p| UserId::from(p.user_id) == *user_id);
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

    async fn issue_payout(&self, user_id: UserId, amount_micro: i64) -> PluginResult<()> {
        if amount_micro <= 0 || self.pot_micro <= 0 {
            return Ok(());
        }
        let repo = LobbyPayoutRepo::new(self.redis.clone());
        match repo.already_paid(self.lobby_id, user_id).await {
            Ok(true) => return Ok(()),
            Ok(false) => {}
            Err(err) => {
                return Err(PluginError::Host(err.to_string()));
            }
        }

        let Some(principal) = self.custodial_address(user_id).await else {
            warn!(
                lobby_id = %self.lobby_id,
                user_id = %user_id,
                "skipping payout; player has no custodial wallet"
            );
            return Ok(());
        };

        let (dest_wallet, game_fee_pct, dest_id, dest_needs_wallet) = self.resolve_dest_fee().await;
        let nonce = repo
            .next_nonce(self.lobby_id)
            .await
            .map_err(|e| PluginError::Host(e.to_string()))?;
        let Some(intent) = build_claim_intent(
            amount_micro,
            game_fee_pct,
            user_id,
            principal,
            dest_wallet,
            dest_id,
            dest_needs_wallet,
        ) else {
            return Ok(());
        };

        let claim = serde_json::json!({
            "userId": intent.user_id.as_uuid().to_string(),
            "principal": intent.principal,
            "amountMicro": intent.amount_micro,
            "nonce": nonce,
            "devWallet": intent.dest_wallet,
            "devFee": intent.dest_fee,
            "devId": intent.dest_id.map(|id| id.as_uuid().to_string()),
            "devNeedsWallet": intent.dest_needs_wallet,
            "role": "place",
        });
        if let Err(err) = repo.push(self.lobby_id, &claim).await {
            error!(
                lobby_id = %self.lobby_id,
                error = %err,
                "failed to persist payout claim"
            );
            return Err(PluginError::Host(err.to_string()));
        }

        let payload = serde_json::json!({
            "lobbyId": self.lobby_id,
            "lobbyPath": self.lobby_path,
            "claim": claim,
        });
        let msg = crate::ws::ServerMessage {
            kind: "lobby.payout".into(),
            payload: payload.clone(),
        };
        let topic = format!("lobby:{}", self.lobby_id);
        self.subscriptions.publish(&self.sessions, &topic, msg);
        self.subscriptions.publish(
            &self.sessions,
            &realtime::user_topic(user_id),
            crate::ws::ServerMessage {
                kind: "lobby.payout".into(),
                payload,
            },
        );
        crate::services::push::spawn_user_notice(
            self.push.clone(),
            self.db.clone(),
            user_id,
            "Prize unlocked".into(),
            "Your share of the pot is ready to claim.".into(),
            format!("/room/{}", self.lobby_path),
        );
        Ok(())
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
            rankings = players.iter().map(|p| UserId::from(p.user_id)).collect();
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
        let won = is_winner;

        let game_id = ctx.game_id.clone().unwrap_or_else(|| self.game_id.clone());

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
