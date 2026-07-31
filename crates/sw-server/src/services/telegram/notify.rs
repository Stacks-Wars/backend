//! Domain-facing Telegram notifications (fire-and-forget, idempotent).

use std::sync::Arc;

use sw_domain::{Lobby, LobbyId, UserId};
use sw_plugin::MatchResult;
use tracing::{info, warn};

use crate::data::lobby_runtime::PlayerStateRepo;
use crate::data::lobbies::PgLobbyRepo;
use crate::data::seasons::{PgSeasonRepo, SeasonRepo};
use crate::data::stats::PgStatsRepo;
use crate::data::telegram::TelegramMsgRepo;
use crate::data::users::PgUserRepo;
use crate::services::vault_oracle::split_pot;
use crate::state::AppState;

use super::client::TelegramClient;
use super::format;

#[derive(Clone)]
pub struct TelegramNotifier {
    pub(super) client: Option<TelegramClient>,
    pub(super) chat_id: i64,
    pub(super) frontend_url: String,
}

impl TelegramNotifier {
    fn room_url(&self, path: &str) -> String {
        format!("{}/room/{}", self.frontend_url, path)
    }

    /// Announce a newly created public lobby.
    pub fn notify_lobby_created(self: &Arc<Self>, state: &AppState, lobby: &Lobby) {
        if !self.enabled() || lobby.is_private {
            return;
        }
        let this = Arc::clone(self);
        let state = state.clone();
        let lobby = lobby.clone();
        tokio::spawn(async move {
            if let Err(err) = this.publish_created(&state, &lobby).await {
                warn!(
                    lobby_id = %lobby.id,
                    error = %err,
                    "telegram lobby.created notify failed"
                );
            }
        });
    }

    /// Post results as a reply to the creation message.
    pub fn notify_lobby_finished(
        self: &Arc<Self>,
        state: &AppState,
        lobby_id: LobbyId,
        result: &MatchResult,
        pot_micro: i64,
        fee_percentage: u8,
    ) {
        self.notify_lobby_finished_parts(
            state.db.clone(),
            state.redis.clone(),
            state.games.clone(),
            lobby_id,
            result,
            pot_micro,
            fee_percentage,
        );
    }

    /// Same as [`Self::notify_lobby_finished`] for callers that hold deps
    /// without a full [`AppState`] (e.g. [`crate::host::ServerGameHost`]).
    pub fn notify_lobby_finished_parts(
        self: &Arc<Self>,
        db: sqlx::PgPool,
        redis: redis::aio::ConnectionManager,
        games: Arc<sw_plugin::GameRegistry>,
        lobby_id: LobbyId,
        result: &MatchResult,
        pot_micro: i64,
        fee_percentage: u8,
    ) {
        if !self.enabled() {
            return;
        }
        let this = Arc::clone(self);
        let result = result.clone();
        tokio::spawn(async move {
            if let Err(err) = this
                .publish_finished(
                    db,
                    redis,
                    games,
                    lobby_id,
                    &result,
                    pot_micro,
                    fee_percentage,
                )
                .await
            {
                warn!(
                    lobby_id = %lobby_id,
                    error = %err,
                    "telegram lobby.finished notify failed"
                );
            }
        });
    }

    /// Remove or mark cancelled the creation announcement.
    pub fn notify_lobby_deleted(self: &Arc<Self>, state: &AppState, lobby: &Lobby) {
        if !self.enabled() {
            return;
        }
        let this = Arc::clone(self);
        let state = state.clone();
        let lobby = lobby.clone();
        tokio::spawn(async move {
            if let Err(err) = this.publish_deleted(&state, &lobby).await {
                warn!(
                    lobby_id = %lobby.id,
                    error = %err,
                    "telegram lobby.deleted notify failed"
                );
            }
        });
    }

    pub async fn reply_leaderboard(
        &self,
        state: &AppState,
        chat_id: i64,
    ) -> anyhow::Result<()> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("telegram disabled"))?;

        let season = PgSeasonRepo::new(state.db.clone())
            .current()
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?
            .ok_or_else(|| anyhow::anyhow!("no active season"))?;
        let (entries, _) = PgStatsRepo::new(state.db.clone())
            .leaderboard_overall(season.id, 10, 0)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        let rows: Vec<(u32, String, i64, i32, i32, i64)> = entries
            .into_iter()
            .map(|e| {
                (
                    e.rank,
                    format::public_name(e.display_name.as_deref(), e.username.as_deref()),
                    e.points,
                    e.total_wins,
                    e.total_matches,
                    e.total_pnl,
                )
            })
            .collect();

        let text = format::leaderboard_html(&season.name, &rows);
        client
            .send_message(chat_id, &text, None, None, None)
            .await?;
        Ok(())
    }

    async fn publish_created(&self, state: &AppState, lobby: &Lobby) -> anyhow::Result<()> {
        let client = self.client.as_ref().expect("enabled");
        let store = TelegramMsgRepo::new(state.redis.clone());
        if !store
            .try_claim_create(lobby.id)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?
        {
            return Ok(());
        }

        let game_name = state
            .games
            .get(&lobby.game_id)
            .map(|f| f.metadata().name)
            .unwrap_or_else(|| lobby.game_id.as_str().to_owned());
        let max_players = state
            .games
            .get(&lobby.game_id)
            .map(|f| f.metadata().max_players)
            .unwrap_or(8);

        let creator = PgUserRepo::new(state.db.clone())
            .get_by_id(lobby.creator_id)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?
            .map(|u| format::public_name(u.display_name.as_deref(), u.username.as_deref()))
            .unwrap_or_else(|| "Host".into());

        let room_url = self.room_url(&lobby.path);
        let text = format::lobby_created_html(
            &lobby.name,
            &game_name,
            &creator,
            lobby.entry_amount_micro,
            lobby.is_sponsored,
            max_players,
        );

        match client
            .send_message(
                self.chat_id,
                &text,
                Some(format::join_keyboard(&room_url)),
                None,
                Some(&room_url),
            )
            .await
        {
            Ok(msg) => {
                store
                    .set_message_id(lobby.id, msg.message_id)
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                info!(lobby_id = %lobby.id, message_id = msg.message_id, "telegram lobby announced");
                Ok(())
            }
            Err(err) => {
                let _ = store.clear(lobby.id).await;
                Err(err)
            }
        }
    }

    async fn publish_finished(
        &self,
        db: sqlx::PgPool,
        redis: redis::aio::ConnectionManager,
        games: Arc<sw_plugin::GameRegistry>,
        lobby_id: LobbyId,
        result: &MatchResult,
        pot_micro: i64,
        fee_percentage: u8,
    ) -> anyhow::Result<()> {
        let client = self.client.as_ref().expect("enabled");
        let store = TelegramMsgRepo::new(redis.clone());
        let Some(record) = store
            .get(lobby_id)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?
        else {
            return Ok(());
        };
        if record.message_id == 0 || record.finished_notified {
            return Ok(());
        }
        // Claim finish slot before send to avoid duplicate replies.
        store
            .set_finished(lobby_id, record.message_id)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        let lobby = PgLobbyRepo::new(db.clone())
            .get_by_id(lobby_id)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?
            .ok_or_else(|| anyhow::anyhow!("lobby missing"))?;

        let game_name = games
            .get(&lobby.game_id)
            .map(|f| f.metadata().name)
            .unwrap_or_else(|| lobby.game_id.as_str().to_owned());

        let mut players = PlayerStateRepo::new(redis.clone())
            .list(lobby_id)
            .await
            .unwrap_or_default();
        players.sort_by_key(|p| p.rank.unwrap_or(usize::MAX));

        let (_, _, winner_share) = split_pot(pot_micro, fee_percentage);
        let winner_ids: Vec<UserId> = if result.winners.is_empty() {
            result.rankings.iter().copied().take(1).collect()
        } else {
            result.winners.clone()
        };

        let mut standings = Vec::new();
        for (idx, p) in players.iter().enumerate() {
            let rank = p.rank.unwrap_or(idx + 1);
            let name = format::public_name(p.display_name.as_deref(), p.username.as_deref());
            let mut prize = p.prize_micro.unwrap_or(0);
            // Prefer on-chain winner share when the engine prize is unset/stale.
            if prize <= 0 && winner_ids.contains(&p.user_id) && winner_share > 0 {
                prize = winner_share;
            }
            standings.push(format::StandingRow {
                rank,
                name,
                prize_micro: prize,
            });
        }
        // If Redis players are gone, fall back to rankings order.
        if standings.is_empty() {
            for (idx, id) in result.rankings.iter().enumerate() {
                let name = if let Ok(Some(u)) =
                    PgUserRepo::new(db.clone()).get_by_id(*id).await
                {
                    format::public_name(u.display_name.as_deref(), u.username.as_deref())
                } else {
                    "Player".into()
                };
                let prize = if winner_ids.contains(id) {
                    winner_share
                } else {
                    0
                };
                standings.push(format::StandingRow {
                    rank: idx + 1,
                    name,
                    prize_micro: prize,
                });
            }
        }

        let room_url = self.room_url(&lobby.path);
        let text = format::lobby_finished_html(
            &lobby.name,
            &game_name,
            pot_micro,
            &standings,
        );

        match client
            .send_message(
                self.chat_id,
                &text,
                None,
                Some(record.message_id),
                Some(&room_url),
            )
            .await
        {
            Ok(_) => {
                info!(lobby_id = %lobby_id, "telegram lobby finished announced");
                Ok(())
            }
            Err(err) => {
                // Allow a later retry if Telegram was temporarily down.
                let _ = store.set_message_id(lobby_id, record.message_id).await;
                Err(err)
            }
        }
    }

    async fn publish_deleted(&self, state: &AppState, lobby: &Lobby) -> anyhow::Result<()> {
        let client = self.client.as_ref().expect("enabled");
        let store = TelegramMsgRepo::new(state.redis.clone());
        let Some(record) = store
            .take(lobby.id)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?
        else {
            return Ok(());
        };
        if record.message_id == 0 {
            return Ok(());
        }

        // Prefer delete; if Telegram rejects (too old / no rights), edit in place.
        if client
            .delete_message(self.chat_id, record.message_id)
            .await
            .is_ok()
        {
            info!(lobby_id = %lobby.id, "telegram lobby announcement deleted");
            return Ok(());
        }

        let game_name = state
            .games
            .get(&lobby.game_id)
            .map(|f| f.metadata().name)
            .unwrap_or_else(|| lobby.game_id.as_str().to_owned());
        let text = format::lobby_cancelled_html(&lobby.name, &game_name);
        match client
            .edit_message_text(self.chat_id, record.message_id, &text)
            .await
        {
            Ok(()) => info!(lobby_id = %lobby.id, "telegram lobby announcement marked cancelled"),
            Err(err) => warn!(lobby_id = %lobby.id, error = %err, "telegram cancel update failed"),
        }
        Ok(())
    }
}
