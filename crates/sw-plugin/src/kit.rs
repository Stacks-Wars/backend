//! Pure game mechanics and result structures (no I/O).

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};
use sw_domain::GameId;
use uuid::Uuid;

/// Per-player in-memory game tracking (separate from lobby [`crate::dto::PlayerStateWire`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GamePlayerState {
    pub user_id: Uuid,
    pub is_eliminated: bool,
    pub position: Option<usize>,
    pub score: i32,
    pub eliminated_at: Option<i64>,
}

impl GamePlayerState {
    pub fn new(user_id: Uuid) -> Self {
        Self {
            user_id,
            is_eliminated: false,
            position: None,
            score: 0,
            eliminated_at: None,
        }
    }

    pub fn eliminate(&mut self) {
        self.is_eliminated = true;
        self.eliminated_at = Some(chrono::Utc::now().timestamp());
    }

    pub fn is_active(&self) -> bool {
        !self.is_eliminated
    }
}

/// Turn rotation with elimination support.
#[derive(Debug, Clone)]
pub struct TurnRotation {
    players: VecDeque<Uuid>,
    current_index: usize,
    eliminated: HashMap<Uuid, bool>,
}

impl TurnRotation {
    pub fn new(player_ids: Vec<Uuid>) -> Self {
        Self {
            players: player_ids.into_iter().collect(),
            current_index: 0,
            eliminated: HashMap::new(),
        }
    }

    pub fn current_player(&self) -> Option<Uuid> {
        self.active_players().get(self.current_index).copied()
    }

    pub fn active_players(&self) -> Vec<Uuid> {
        self.players
            .iter()
            .filter(|id| !self.eliminated.get(id).unwrap_or(&false))
            .copied()
            .collect()
    }

    pub fn active_count(&self) -> usize {
        self.active_players().len()
    }

    pub fn next_turn(&mut self) -> Option<Uuid> {
        if self.active_count() == 0 {
            return None;
        }

        let active = self.active_players();
        self.current_index = (self.current_index + 1) % active.len();
        active.get(self.current_index).copied()
    }

    pub fn eliminate_player(&mut self, player_id: Uuid) {
        self.eliminated.insert(player_id, true);

        let active_count = self.active_count();
        if active_count > 0 && self.current_index >= active_count {
            self.current_index = self.current_index % active_count;
        }
    }

    pub fn is_game_over(&self) -> bool {
        self.active_count() <= 1
    }

    pub fn get_winner(&self) -> Option<Uuid> {
        let active = self.active_players();
        if active.len() == 1 {
            active.first().copied()
        } else {
            None
        }
    }
}

/// Per-player time bank. Only the active player's remaining time ticks.
///
/// `start_turn` on the already-active player is a no-op so multi-jump
/// continuations keep one continuous clock.
#[derive(Debug, Clone)]
pub struct PlayerClocks {
    remaining: HashMap<Uuid, Duration>,
    active: Option<Uuid>,
    turn_started: Option<Instant>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ClockReading {
    pub user_id: Uuid,
    pub remaining_ms: u64,
}

impl PlayerClocks {
    pub fn new(player_ids: impl IntoIterator<Item = Uuid>, bank: Duration) -> Self {
        let remaining = player_ids.into_iter().map(|id| (id, bank)).collect();
        Self {
            remaining,
            active: None,
            turn_started: None,
        }
    }

    pub fn start_turn(&mut self, id: Uuid) {
        if self.active == Some(id) {
            return;
        }
        self.pause_turn();
        self.active = Some(id);
        self.turn_started = Some(Instant::now());
    }

    pub fn pause_turn(&mut self) {
        let Some(id) = self.active.take() else {
            return;
        };
        let elapsed = self
            .turn_started
            .take()
            .map(|started| started.elapsed())
            .unwrap_or_default();
        if let Some(left) = self.remaining.get_mut(&id) {
            *left = left.saturating_sub(elapsed);
        }
    }

    pub fn remaining(&self, id: Uuid) -> Duration {
        let stored = self.remaining.get(&id).copied().unwrap_or_default();
        if self.active != Some(id) {
            return stored;
        }
        let elapsed = self
            .turn_started
            .map(|started| started.elapsed())
            .unwrap_or_default();
        stored.saturating_sub(elapsed)
    }

    pub fn remaining_ms(&self, id: Uuid) -> u64 {
        u64::try_from(self.remaining(id).as_millis()).unwrap_or(u64::MAX)
    }

    pub fn active(&self) -> Option<Uuid> {
        self.active
    }

    pub fn flagged(&self) -> Option<Uuid> {
        let id = self.active?;
        (self.remaining(id).is_zero()).then_some(id)
    }

    /// Wall-clock instant when the active bank hits zero. Clients interpolate
    /// from this instead of waiting on 1s server ticks.
    pub fn deadline_unix_ms(&self) -> Option<i64> {
        let id = self.active?;
        let left = self.remaining(id);
        Some(chrono::Utc::now().timestamp_millis() + left.as_millis() as i64)
    }

    pub fn readings(&self) -> Vec<ClockReading> {
        let mut ids: Vec<Uuid> = self.remaining.keys().copied().collect();
        ids.sort_unstable();
        ids.into_iter()
            .map(|user_id| ClockReading {
                user_id,
                remaining_ms: self.remaining_ms(user_id),
            })
            .collect()
    }
}

/// Final rankings returned to clients / persistence layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameResults {
    pub rankings: Vec<PlayerRanking>,
    pub finished_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerRanking {
    pub user_id: Uuid,
    pub rank: usize,
    pub score: Option<i32>,
    pub prize: Option<f64>,
}

impl GameResults {
    pub fn from_ordered_players(player_ids: Vec<Uuid>) -> Self {
        let rankings = player_ids
            .into_iter()
            .enumerate()
            .map(|(idx, user_id)| PlayerRanking {
                user_id,
                rank: idx + 1,
                score: None,
                prize: None,
            })
            .collect();

        Self {
            rankings,
            finished_at: chrono::Utc::now().timestamp(),
            metadata: None,
        }
    }

    pub fn from_game_states(mut states: Vec<GamePlayerState>) -> Self {
        states.sort_by(|a, b| {
            match (a.is_eliminated, b.is_eliminated) {
                (false, true) => std::cmp::Ordering::Less,
                (true, false) => std::cmp::Ordering::Greater,
                (false, false) => std::cmp::Ordering::Equal,
                (true, true) => b.eliminated_at.cmp(&a.eliminated_at),
            }
        });

        let rankings = states
            .into_iter()
            .enumerate()
            .map(|(idx, state)| PlayerRanking {
                user_id: state.user_id,
                rank: idx + 1,
                score: Some(state.score),
                prize: None,
            })
            .collect();

        Self {
            rankings,
            finished_at: chrono::Utc::now().timestamp(),
            metadata: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameBootstrap<TState> {
    pub game_id: Uuid,
    pub status: GameStatus,
    pub current_state: TState,
    pub players: Vec<Uuid>,
    pub started_at: i64,
    pub finished_at: Option<i64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum GameStatus {
    InProgress,
    Finished,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameSummary {
    pub results: GameResults,
    pub metadata: serde_json::Value,
    pub finished_at: i64,
}

#[derive(Debug, Clone)]
pub struct PlayerResult {
    pub rank: usize,
    pub prize: Option<f64>,
    pub wars_point: i64,
}

/// Context for calculating wars points for a player result.
#[derive(Debug, Clone)]
pub struct WarsPointContext {
    pub user_id: Uuid,
    pub game_id: Option<GameId>,
    pub rank: usize,
    pub prize: Option<f64>,
    pub participants: usize,
    pub entry_amount: Option<f64>,
    pub current_amount: Option<f64>,
    pub is_sponsored: bool,
    pub creator_id: Option<Uuid>,
    pub active_players: usize,
    pub token_symbol: Option<String>,
    pub token_contract_id: Option<String>,
}

/// Match Wars Points. `is_winner` is the engine's winner flag (draws get no win bonus).
pub fn calculate_wars_point_for(ctx: &WarsPointContext, is_winner: bool) -> i64 {
    let rank = ctx.rank.max(1) as i64;
    let participants = ctx.participants.max(1) as i64;
    let mut points = 5;
    points += (participants - rank).max(0) * 2;
    if is_winner {
        points += 8;
    }
    let paid = !ctx.is_sponsored && ctx.entry_amount.unwrap_or(0.0) > 0.0;
    if paid {
        points += 3;
    }
    points.clamp(0, 40)
}

/// Fallback used by engines when the host save fails. Treats rank 1 as a win.
pub fn calculate_wars_point(ctx: &WarsPointContext) -> i64 {
    calculate_wars_point_for(ctx, ctx.rank == 1)
}

/// Optional first-party placement split. Other games can ignore this and
/// pay whatever they put on `WarsPointContext.prize`.
///
/// - 2 players: 70 / 30
/// - 3+ players: 50 / 30 / 20 for 1st–3rd
pub fn placement_share_pct(rank: usize, participants: usize) -> u8 {
    if participants <= 2 {
        match rank {
            1 => 70,
            2 => 30,
            _ => 0,
        }
    } else {
        match rank {
            1 => 50,
            2 => 30,
            3 => 20,
            _ => 0,
        }
    }
}

/// Prize in the same unit as `pot` (typically dollars). `None` when unpaid.
pub fn placement_prize(pot: f64, rank: usize, participants: usize) -> Option<f64> {
    let pct = placement_share_pct(rank, participants);
    if pct == 0 || pot <= 0.0 {
        None
    } else {
        Some((pot * f64::from(pct)) / 100.0)
    }
}

/// How many paid places this roster size fills (2 heads-up, else 3).
pub fn paid_place_count(participants: usize) -> usize {
    if participants <= 2 { 2 } else { 3 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turn_rotation() {
        let players = vec![Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()];
        let mut rotation = TurnRotation::new(players.clone());

        assert_eq!(rotation.current_player(), Some(players[0]));
        assert_eq!(rotation.active_count(), 3);

        rotation.next_turn();
        assert_eq!(rotation.current_player(), Some(players[1]));

        rotation.eliminate_player(players[1]);
        assert_eq!(rotation.active_count(), 2);
        assert_eq!(rotation.current_player(), Some(players[2]));

        rotation.eliminate_player(players[2]);
        assert_eq!(rotation.active_count(), 1);
        assert!(rotation.is_game_over());
        assert_eq!(rotation.get_winner(), Some(players[0]));
    }

    #[test]
    fn clocks_pause_resume_and_idempotent_start() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let mut clocks = PlayerClocks::new([a, b], Duration::from_millis(1_000));

        clocks.start_turn(a);
        std::thread::sleep(Duration::from_millis(30));
        let ticking = clocks.remaining(a);
        assert!(ticking < Duration::from_millis(1_000));
        assert_eq!(clocks.remaining(b), Duration::from_millis(1_000));

        clocks.start_turn(a);
        assert!(clocks.remaining(a) <= ticking);

        clocks.pause_turn();
        let paused = clocks.remaining(a);
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(clocks.remaining(a), paused);

        clocks.start_turn(b);
        assert_eq!(clocks.remaining(a), paused);
        assert_eq!(clocks.active(), Some(b));
    }

    #[test]
    fn clocks_flag_when_active_bank_hits_zero() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let mut clocks = PlayerClocks::new([a, b], Duration::from_millis(5));
        clocks.start_turn(a);
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(clocks.flagged(), Some(a));
        assert!(clocks.remaining(a).is_zero());
        assert_eq!(clocks.remaining(b), Duration::from_millis(5));
        assert!(clocks.deadline_unix_ms().is_some());
    }

    #[test]
    fn placement_split_heads_up_and_field() {
        assert_eq!(placement_share_pct(1, 2), 70);
        assert_eq!(placement_share_pct(2, 2), 30);
        assert_eq!(placement_share_pct(3, 2), 0);
        assert_eq!(placement_share_pct(1, 4), 50);
        assert_eq!(placement_share_pct(2, 4), 30);
        assert_eq!(placement_share_pct(3, 4), 20);
        assert_eq!(placement_share_pct(4, 4), 0);
        assert_eq!(placement_prize(10.0, 1, 2), Some(7.0));
        assert_eq!(placement_prize(10.0, 3, 3), Some(2.0));
        assert_eq!(paid_place_count(2), 2);
        assert_eq!(paid_place_count(5), 3);
    }

    fn ctx(rank: usize, participants: usize, paid: bool, sponsored: bool) -> WarsPointContext {
        WarsPointContext {
            user_id: Uuid::new_v4(),
            game_id: None,
            rank,
            prize: None,
            participants,
            entry_amount: if paid { Some(1.0) } else { None },
            current_amount: None,
            is_sponsored: sponsored,
            creator_id: None,
            active_players: participants,
            token_symbol: None,
            token_contract_id: None,
        }
    }

    #[test]
    fn wars_points_heads_up_and_paid() {
        assert_eq!(calculate_wars_point_for(&ctx(2, 2, false, false), false), 5);
        assert_eq!(calculate_wars_point_for(&ctx(1, 2, false, false), true), 15);
        assert_eq!(calculate_wars_point_for(&ctx(1, 2, true, false), true), 18);
        assert_eq!(calculate_wars_point_for(&ctx(1, 4, false, false), true), 19);
        assert_eq!(calculate_wars_point_for(&ctx(1, 2, true, true), true), 15);
        assert_eq!(calculate_wars_point(&ctx(1, 2, false, false)), 15);
    }
}
