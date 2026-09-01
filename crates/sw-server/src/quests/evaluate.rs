//! Bucket qualifying matches into period metrics and map the catalog.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, NaiveDate, Utc};
use uuid::Uuid;

use super::catalog::{self, Metric, QuestDef};
use super::period::{self, PeriodClock, PeriodKind};
use super::streak::{self, Streak};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum QuestState {
    Locked,
    Active,
    Claimable,
    Claimed,
}

#[derive(Debug, Clone)]
pub struct QualifyingMatch {
    pub game_id: String,
    pub finished_at: DateTime<Utc>,
    pub is_winner: bool,
    pub entry_micro: i64,
    pub creator_id: Uuid,
    pub player_count: i32,
    pub opponents: Vec<Uuid>,
}

#[derive(Debug, Clone, Default)]
pub struct PeriodBucket {
    pub games_played: i64,
    pub games_won: i64,
    pub unique_games: HashSet<String>,
    pub unique_opponents: HashSet<Uuid>,
    pub paid_games: i64,
    pub paid_entry_micro: i64,
    pub active_days: HashSet<NaiveDate>,
    pub hosted: i64,
    pub joined: i64,
}

impl PeriodBucket {
    fn add(&mut self, row: &QualifyingMatch, user_id: Uuid) {
        if row.player_count < 2 {
            return;
        }
        self.games_played += 1;
        if row.is_winner {
            self.games_won += 1;
        }
        self.unique_games.insert(row.game_id.clone());
        for opp in &row.opponents {
            if *opp != user_id {
                self.unique_opponents.insert(*opp);
            }
        }
        if row.entry_micro > 0 {
            self.paid_games += 1;
            self.paid_entry_micro += row.entry_micro;
        }
        self.active_days.insert(row.finished_at.date_naive());
        if row.creator_id == user_id {
            self.hosted += 1;
        } else {
            self.joined += 1;
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct GettingStartedActions {
    pub username_set: bool,
    pub hosted: bool,
    pub joined: bool,
    pub won: bool,
}

impl GettingStartedActions {
    pub fn all_done(self) -> bool {
        self.username_set && self.hosted && self.joined && self.won
    }
}

#[derive(Debug, Clone, Default)]
pub struct Extras {
    pub getting_started: GettingStartedActions,
    pub referral_successes: i64,
    pub daily_claims_in_week: i64,
    pub daily_claims_in_month: i64,
    pub any_claims_in_season: i64,
    pub season_streak: Streak,
}

#[derive(Debug, Clone)]
pub struct OpenPeriods {
    pub daily: PeriodClock,
    pub weekly: PeriodClock,
    pub monthly: PeriodClock,
    pub seasonal: Option<PeriodClock>,
    pub paid: Option<PeriodClock>,
    pub lifetime: PeriodClock,
}

impl OpenPeriods {
    pub fn current(
        now: DateTime<Utc>,
        season_id: Option<i32>,
        season_start: Option<DateTime<Utc>>,
        season_end: Option<DateTime<Utc>>,
    ) -> Self {
        let seasonal = match (season_id, season_start, season_end) {
            (Some(id), Some(start), Some(end)) => Some(period::seasonal(id, start, end)),
            _ => None,
        };
        let paid = seasonal.clone().map(|mut clock| {
            clock.kind = PeriodKind::PaidLadder;
            clock
        });
        Self {
            daily: period::daily(now),
            weekly: period::weekly(now),
            monthly: period::monthly(now),
            seasonal,
            paid,
            lifetime: period::lifetime(),
        }
    }

    pub fn covering_start(&self) -> DateTime<Utc> {
        let mut start = self.weekly.starts_at.min(self.monthly.starts_at);
        if let Some(season) = &self.seasonal {
            start = start.min(season.starts_at);
        }
        start
    }

    pub fn clock_for(&self, kind: PeriodKind) -> Option<&PeriodClock> {
        match kind {
            PeriodKind::GettingStarted => Some(&self.lifetime),
            PeriodKind::Daily => Some(&self.daily),
            PeriodKind::Weekly => Some(&self.weekly),
            PeriodKind::Monthly => Some(&self.monthly),
            PeriodKind::Seasonal => self.seasonal.as_ref(),
            PeriodKind::PaidLadder => self.paid.as_ref(),
        }
    }

    pub fn daily_ids_in_week(&self) -> Vec<String> {
        (0..7)
            .map(|i| {
                (self.weekly.starts_at + chrono::Duration::days(i))
                    .date_naive()
                    .format("%Y-%m-%d")
                    .to_string()
            })
            .collect()
    }

    pub fn daily_ids_in_month(&self) -> Vec<String> {
        let start = self.monthly.starts_at.date_naive();
        let end = self
            .monthly
            .resets_at
            .map(|d| d.date_naive())
            .unwrap_or(start);
        let mut ids = Vec::new();
        let mut day = start;
        while day < end {
            ids.push(day.format("%Y-%m-%d").to_string());
            day += chrono::Duration::days(1);
        }
        ids
    }
}

#[derive(Debug, Clone, Default)]
pub struct Buckets {
    inner: HashMap<PeriodKind, PeriodBucket>,
}

impl Buckets {
    pub fn from_matches(user_id: Uuid, rows: &[QualifyingMatch], periods: &OpenPeriods) -> Self {
        let mut inner: HashMap<PeriodKind, PeriodBucket> = HashMap::new();
        for row in rows {
            if row.player_count < 2 {
                continue;
            }
            for clock in [
                Some(&periods.daily),
                Some(&periods.weekly),
                Some(&periods.monthly),
                periods.seasonal.as_ref(),
            ]
            .into_iter()
            .flatten()
            {
                if in_window(row.finished_at, clock) {
                    inner.entry(clock.kind).or_default().add(row, user_id);
                }
            }
        }
        Self { inner }
    }

    pub fn get(&self, kind: PeriodKind) -> PeriodBucket {
        self.inner.get(&kind).cloned().unwrap_or_default()
    }
}

fn in_window(at: DateTime<Utc>, clock: &PeriodClock) -> bool {
    at >= clock.starts_at && clock.resets_at.is_none_or(|end| at < end)
}

/// Paid volume that counts toward the current bonus mission stage.
///
/// Independent of weekly / monthly / seasonal `PaidEntryMicro` buckets.
/// `after` is the previous stage's `claimed_at`: only matches strictly later
/// count, so progress never carries from one stage into the next.
pub fn bonus_paid_micro(
    matches: &[QualifyingMatch],
    clock: &PeriodClock,
    after: Option<DateTime<Utc>>,
) -> i64 {
    matches
        .iter()
        .filter(|row| row.player_count >= 2 && row.entry_micro > 0)
        .filter(|row| in_window(row.finished_at, clock))
        .filter(|row| after.is_none_or(|t| row.finished_at > t))
        .map(|row| row.entry_micro)
        .sum()
}

pub fn season_dates(rows: &[QualifyingMatch], seasonal: Option<&PeriodClock>) -> Vec<NaiveDate> {
    let Some(clock) = seasonal else {
        return Vec::new();
    };
    let mut dates: Vec<NaiveDate> = rows
        .iter()
        .filter(|row| row.player_count >= 2 && in_window(row.finished_at, clock))
        .map(|row| row.finished_at.date_naive())
        .collect();
    dates.sort_unstable();
    dates.dedup();
    dates
}

pub fn metric_value(
    def: &QuestDef,
    buckets: &Buckets,
    extras: &Extras,
    registered_games: usize,
) -> i64 {
    let bucket = buckets.get(def.category);
    let raw = match def.metric {
        Metric::UsernameSet => i64::from(extras.getting_started.username_set),
        Metric::HostedGames => i64::from(extras.getting_started.hosted),
        Metric::JoinedGames => i64::from(extras.getting_started.joined),
        Metric::GamesPlayed => bucket.games_played,
        Metric::GamesWon => {
            if def.category == PeriodKind::GettingStarted {
                i64::from(extras.getting_started.won)
            } else {
                bucket.games_won
            }
        }
        Metric::UniqueGames => bucket.unique_games.len() as i64,
        Metric::UniqueOpponents => bucket.unique_opponents.len() as i64,
        Metric::PaidGames => bucket.paid_games,
        Metric::PaidEntryMicro => bucket.paid_entry_micro,
        Metric::ActiveDays => bucket.active_days.len() as i64,
        Metric::DailyClaims => match def.category {
            PeriodKind::Weekly => extras.daily_claims_in_week,
            PeriodKind::Monthly => extras.daily_claims_in_month,
            _ => 0,
        },
        Metric::AnyClaims => extras.any_claims_in_season,
        Metric::SuccessfulReferrals => extras.referral_successes,
        Metric::LongestStreak => extras.season_streak.longest as i64,
    };
    let target = def.target.value(registered_games);
    raw.min(target)
}

pub fn quest_state(
    def: &QuestDef,
    progress: i64,
    target: i64,
    claimed_ids: &HashSet<String>,
    period_id: &str,
) -> QuestState {
    let key = claim_key(def.id, period_id);
    if claimed_ids.contains(&key) {
        return QuestState::Claimed;
    }
    if let Some(index) = catalog::paid_stage_index(def.id)
        && let Some(prev) = catalog::previous_paid_id(index)
        && !claimed_ids.contains(&claim_key(prev, period_id))
    {
        return QuestState::Locked;
    }
    if progress >= target && target > 0 {
        QuestState::Claimable
    } else {
        QuestState::Active
    }
}

/// Claim uniqueness is `(user_id, quest_id, period_id)`.
pub fn claim_key(quest_id: &str, period_id: &str) -> String {
    format!("{quest_id}:{period_id}")
}

pub fn claimed_set(rows: &[(String, String)]) -> HashSet<String> {
    rows.iter()
        .map(|(quest_id, period_id)| claim_key(quest_id, period_id))
        .collect()
}

pub fn streak_from_matches(rows: &[QualifyingMatch], seasonal: Option<&PeriodClock>) -> Streak {
    streak::from_sorted_unique_dates(&season_dates(rows, seasonal))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(day: u32, hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, day, hour, 0, 0).unwrap()
    }

    fn match_row(
        game: &str,
        day: u32,
        winner: bool,
        creator: Uuid,
        opponents: Vec<Uuid>,
        paid: bool,
        player_count: i32,
    ) -> QualifyingMatch {
        QualifyingMatch {
            game_id: game.into(),
            finished_at: at(day, 12),
            is_winner: winner,
            entry_micro: if paid { 1_000_000 } else { 0 },
            creator_id: creator,
            player_count,
            opponents,
        }
    }

    #[test]
    fn ignores_solo_matches() {
        let me = Uuid::now_v7();
        let periods = OpenPeriods::current(at(1, 15), None, None, None);
        let rows = [match_row("checkers", 1, true, me, vec![], false, 1)];
        let buckets = Buckets::from_matches(me, &rows, &periods);
        assert_eq!(buckets.get(PeriodKind::Daily).games_played, 0);
    }

    #[test]
    fn hosted_vs_joined() {
        let me = Uuid::now_v7();
        let other = Uuid::now_v7();
        let periods = OpenPeriods::current(at(1, 15), None, None, None);
        let rows = [
            match_row("checkers", 1, false, me, vec![other], false, 2),
            match_row("ludo", 1, true, other, vec![me], false, 2),
        ];
        let buckets = Buckets::from_matches(me, &rows, &periods);
        let daily = buckets.get(PeriodKind::Daily);
        assert_eq!(daily.hosted, 1);
        assert_eq!(daily.joined, 1);
        assert_eq!(daily.games_won, 1);
        assert_eq!(daily.unique_games.len(), 2);
        assert_eq!(daily.unique_opponents.len(), 1);
    }

    #[test]
    fn paid_stage_locked_until_previous_claimed() {
        let def = catalog::get("paid.volume:10").unwrap();
        let claimed = HashSet::new();
        let state = quest_state(&def, 10_000_000, 10_000_000, &claimed, "season:3");
        assert_eq!(state, QuestState::Locked);

        let mut claimed = HashSet::new();
        claimed.insert(claim_key("paid.volume:5", "season:3"));
        let state = quest_state(&def, 10_000_000, 10_000_000, &claimed, "season:3");
        assert_eq!(state, QuestState::Claimable);
    }

    #[test]
    fn bonus_stage_progress_does_not_carry() {
        let other = Uuid::now_v7();
        let me = Uuid::now_v7();
        let season_start = Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap();
        let season_end = Utc.with_ymd_and_hms(2026, 12, 1, 0, 0, 0).unwrap();
        let clock = period::paid_ladder(3, season_start, season_end);
        let rows = [
            match_row("checkers", 1, false, me, vec![other], true, 2),
            match_row("checkers", 2, false, me, vec![other], true, 2),
            match_row("checkers", 2, false, me, vec![other], true, 2),
            match_row("checkers", 2, false, me, vec![other], true, 2),
            match_row("checkers", 2, false, me, vec![other], true, 2),
        ];
        // Five $1 matches before the $5 claim — enough for stage 1, not carried forward.
        assert_eq!(bonus_paid_micro(&rows, &clock, None), 5_000_000);

        let claimed_at = at(2, 13);
        assert_eq!(bonus_paid_micro(&rows, &clock, Some(claimed_at)), 0);

        let mut next = rows.to_vec();
        next.push(QualifyingMatch {
            game_id: "ludo".into(),
            finished_at: at(3, 12),
            is_winner: false,
            entry_micro: 10_000_000,
            creator_id: me,
            player_count: 2,
            opponents: vec![other],
        });
        assert_eq!(
            bonus_paid_micro(&next, &clock, Some(claimed_at)),
            10_000_000
        );
    }

    #[test]
    fn bonus_progress_is_not_the_weekly_paid_bucket() {
        let me = Uuid::now_v7();
        let other = Uuid::now_v7();
        let periods = OpenPeriods::current(at(1, 15), Some(3), Some(at(1, 0)), Some(at(30, 0)));
        let rows = [match_row("checkers", 1, false, me, vec![other], true, 2)];
        let buckets = Buckets::from_matches(me, &rows, &periods);
        assert_eq!(buckets.get(PeriodKind::Weekly).paid_entry_micro, 1_000_000);
        assert_eq!(buckets.get(PeriodKind::Monthly).paid_entry_micro, 1_000_000);
        assert_eq!(
            buckets.get(PeriodKind::Seasonal).paid_entry_micro,
            1_000_000
        );
        assert_eq!(buckets.get(PeriodKind::PaidLadder).paid_entry_micro, 0);
        let clock = periods.paid.as_ref().unwrap();
        assert_eq!(bonus_paid_micro(&rows, clock, None), 1_000_000);
    }

    #[test]
    fn getting_started_progress_is_actions_not_claims() {
        let extras = Extras {
            getting_started: GettingStartedActions {
                username_set: true,
                hosted: true,
                joined: true,
                won: true,
            },
            ..Default::default()
        };
        assert!(extras.getting_started.all_done());
        let def = catalog::get("gs.username").unwrap();
        let buckets = Buckets::default();
        assert_eq!(metric_value(&def, &buckets, &extras, 4), 1);
        let state = quest_state(&def, 1, 1, &HashSet::new(), "lifetime");
        assert_eq!(state, QuestState::Claimable);
    }

    #[test]
    fn referral_metric_uses_successful_count() {
        let def = catalog::get("weekly.referral-1").unwrap();
        let extras = Extras {
            referral_successes: 0,
            ..Default::default()
        };
        assert_eq!(metric_value(&def, &Buckets::default(), &extras, 4), 0);
        let extras = Extras {
            referral_successes: 1,
            ..Default::default()
        };
        assert_eq!(metric_value(&def, &Buckets::default(), &extras, 4), 1);
    }

    #[test]
    fn unique_games_target_follows_registry() {
        let def = catalog::get("monthly.games-all").unwrap();
        assert_eq!(def.target.value(4), 4);
        assert_eq!(def.target.value(2), 2);
    }
}
