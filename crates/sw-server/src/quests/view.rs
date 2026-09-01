//! Assembled `GET /quests/me` payload.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sw_domain::{Season, UserId};

use crate::data::quest_claims::QuestClaimRow;
use crate::data::users::QuestFlags;
use crate::quests::catalog::{self, QuestDef};
use crate::quests::evaluate::{
    Buckets, Extras, OpenPeriods, QualifyingMatch, QuestState, bonus_paid_micro, claimed_set,
    metric_value, quest_state, streak_from_matches,
};
use crate::quests::period::PeriodClock;
use crate::quests::streak::Streak;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CtaView {
    pub href: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeriodView {
    pub kind: crate::quests::period::PeriodKind,
    pub id: String,
    pub starts_at: DateTime<Utc>,
    pub resets_at: Option<DateTime<Utc>>,
    pub resets_label: String,
}

impl From<&PeriodClock> for PeriodView {
    fn from(clock: &PeriodClock) -> Self {
        Self {
            kind: clock.kind,
            id: clock.id.clone(),
            starts_at: clock.starts_at,
            resets_at: clock.resets_at,
            resets_label: "Resets 00:00 UTC".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestView {
    pub id: String,
    pub title: String,
    pub description: String,
    pub category: crate::quests::period::PeriodKind,
    pub progress: i64,
    pub target: i64,
    pub state: QuestState,
    pub reward_points: i32,
    pub cta: CtaView,
    pub period_id: String,
    pub resets_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestMeResponse {
    pub catalog_version: i32,
    pub now: DateTime<Utc>,
    pub periods: Vec<PeriodView>,
    pub streak: StreakView,
    pub getting_started_completed: bool,
    pub getting_started_completed_at: Option<DateTime<Utc>>,
    pub referral_prompt_status: String,
    pub quest_intro_seen_at: Option<DateTime<Utc>>,
    pub successful_referrals: i64,
    #[serde(default)]
    pub season_quest_points: i64,
    pub quests: Vec<QuestView>,
    #[serde(default)]
    pub bonus_mission: Option<BonusMissionView>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BonusMissionView {
    #[serde(flatten)]
    pub quest: QuestView,
    pub stage_index: i32,
    pub stage_count: i32,
    pub dollars: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreakView {
    pub current: i32,
    pub longest: i32,
    pub last_active_date: Option<String>,
}

impl From<Streak> for StreakView {
    fn from(s: Streak) -> Self {
        Self {
            current: s.current,
            longest: s.longest,
            last_active_date: s.last_active_date.map(|d| d.to_string()),
        }
    }
}

pub struct AssembleInput<'a> {
    pub user_id: UserId,
    pub now: DateTime<Utc>,
    pub season: Option<&'a Season>,
    pub registered_games: usize,
    pub matches: &'a [QualifyingMatch],
    pub claims: &'a [QuestClaimRow],
    pub flags: &'a QuestFlags,
    pub extras: Extras,
}

pub fn assemble(input: AssembleInput<'_>) -> QuestMeResponse {
    let season_id = input.season.map(|s| s.id.as_i32());
    let periods = OpenPeriods::current(
        input.now,
        season_id,
        input.season.map(|s| s.starts_at),
        input.season.map(|s| s.ends_at),
    );
    let buckets = Buckets::from_matches(input.user_id.as_uuid(), input.matches, &periods);
    let claimed = claimed_set(
        &input
            .claims
            .iter()
            .map(|c| (c.quest_id.clone(), c.period_id.clone()))
            .collect::<Vec<_>>(),
    );
    let mut extras = input.extras;
    extras.season_streak = streak_from_matches(input.matches, periods.seasonal.as_ref());
    extras.getting_started.username_set |= input.flags.username_set;

    let mut period_views = vec![
        PeriodView::from(&periods.lifetime),
        PeriodView::from(&periods.daily),
        PeriodView::from(&periods.weekly),
        PeriodView::from(&periods.monthly),
    ];
    if let Some(season) = &periods.seasonal {
        period_views.push(PeriodView::from(season));
    }

    let mut quests = Vec::new();
    for def in catalog::all_defs() {
        if def.category == crate::quests::period::PeriodKind::PaidLadder {
            continue;
        }
        let Some(clock) = periods.clock_for(def.category) else {
            continue;
        };
        quests.push(view_for(
            &def,
            clock,
            &buckets,
            &extras,
            &claimed,
            input.registered_games,
        ));
    }

    let bonus_mission =
        current_bonus_mission(periods.paid.as_ref(), input.matches, input.claims, &claimed);

    let season_quest_points = input
        .claims
        .iter()
        .filter(|c| season_id.is_some() && c.season_id == season_id)
        .map(|c| i64::from(c.reward_points))
        .sum();

    QuestMeResponse {
        catalog_version: catalog::VERSION,
        now: input.now,
        periods: period_views,
        streak: extras.season_streak.into(),
        getting_started_completed: extras.getting_started.all_done()
            || input.flags.getting_started_completed_at.is_some(),
        getting_started_completed_at: input.flags.getting_started_completed_at,
        referral_prompt_status: input.flags.referral_prompt_status.clone(),
        quest_intro_seen_at: input.flags.quest_intro_seen_at,
        successful_referrals: extras.referral_successes,
        season_quest_points,
        quests,
        bonus_mission,
    }
}

fn view_for(
    def: &QuestDef,
    clock: &PeriodClock,
    buckets: &Buckets,
    extras: &Extras,
    claimed: &std::collections::HashSet<String>,
    registered_games: usize,
) -> QuestView {
    let target = def.target.value(registered_games);
    let progress = metric_value(def, buckets, extras, registered_games);
    let state = quest_state(def, progress, target, claimed, &clock.id);
    quest_view(def, clock, progress, target, state)
}

fn quest_view(
    def: &QuestDef,
    clock: &PeriodClock,
    progress: i64,
    target: i64,
    state: QuestState,
) -> QuestView {
    QuestView {
        id: def.id.to_owned(),
        title: def.title.to_owned(),
        description: def.description.to_owned(),
        category: def.category,
        progress,
        target,
        state,
        reward_points: def.reward_points,
        cta: CtaView {
            href: def.cta.href.to_owned(),
            label: def.cta.label.to_owned(),
        },
        period_id: clock.id.clone(),
        resets_at: clock.resets_at,
    }
}

fn current_bonus_mission(
    clock: Option<&PeriodClock>,
    matches: &[QualifyingMatch],
    claims: &[QuestClaimRow],
    claimed: &std::collections::HashSet<String>,
) -> Option<BonusMissionView> {
    let clock = clock?;
    let stage_count = catalog::PAID_STAGES.len() as i32;
    for (index, def) in catalog::paid_defs().into_iter().enumerate() {
        if claimed.contains(&crate::quests::evaluate::claim_key(def.id, &clock.id)) {
            continue;
        }
        let after = catalog::previous_paid_id(index).and_then(|prev| {
            claims
                .iter()
                .find(|c| c.quest_id == prev && c.period_id == clock.id)
                .map(|c| c.claimed_at)
        });
        let target = def.target.value(1);
        let progress = bonus_paid_micro(matches, clock, after).min(target);
        let state = quest_state(&def, progress, target, claimed, &clock.id);
        return Some(BonusMissionView {
            quest: quest_view(&def, clock, progress, target, state),
            stage_index: index as i32,
            stage_count,
            dollars: catalog::PAID_STAGES[index].0,
        });
    }
    None
}

pub fn quest_view_for<'a>(me: &'a QuestMeResponse, quest_id: &str) -> Option<&'a QuestView> {
    me.quests.iter().find(|q| q.id == quest_id).or_else(|| {
        me.bonus_mission
            .as_ref()
            .filter(|b| b.quest.id == quest_id)
            .map(|b| &b.quest)
    })
}

pub fn paid_period_id(me: &QuestMeResponse) -> Option<&str> {
    me.bonus_mission
        .as_ref()
        .map(|b| b.quest.period_id.as_str())
        .or_else(|| {
            me.periods
                .iter()
                .find(|p| {
                    matches!(
                        p.kind,
                        crate::quests::period::PeriodKind::Seasonal
                            | crate::quests::period::PeriodKind::PaidLadder
                    )
                })
                .map(|p| p.id.as_str())
        })
}

pub fn is_claimable<'a>(me: &'a QuestMeResponse, quest_id: &str) -> Option<&'a QuestView> {
    quest_view_for(me, quest_id).filter(|q| q.state == QuestState::Claimable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use sw_domain::SeasonId;
    use uuid::Uuid;

    use crate::quests::evaluate::QuestState;

    fn at(day: u32, hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, day, hour, 0, 0).unwrap()
    }

    fn season() -> Season {
        Season {
            id: SeasonId(3),
            name: "Test".into(),
            description: None,
            starts_at: at(1, 0),
            ends_at: Utc.with_ymd_and_hms(2026, 12, 1, 0, 0, 0).unwrap(),
            created_at: at(1, 0),
        }
    }

    fn flags() -> QuestFlags {
        QuestFlags {
            username_set: false,
            referral_prompt_status: "skipped".into(),
            quest_intro_seen_at: None,
            getting_started_completed_at: None,
        }
    }

    fn paid_match(me: Uuid, other: Uuid, day: u32, hour: u32, micro: i64) -> QualifyingMatch {
        QualifyingMatch {
            game_id: "checkers".into(),
            finished_at: at(day, hour),
            is_winner: false,
            entry_micro: micro,
            creator_id: me,
            player_count: 2,
            opponents: vec![other],
        }
    }

    fn claim_row(
        quest_id: &str,
        period_id: &str,
        at_time: DateTime<Utc>,
        points: i32,
    ) -> QuestClaimRow {
        QuestClaimRow {
            id: Uuid::now_v7(),
            user_id: Uuid::now_v7(),
            quest_id: quest_id.into(),
            period_kind: "paid_ladder".into(),
            period_id: period_id.into(),
            season_id: Some(3),
            reward_points: points,
            catalog_version: 1,
            claimed_at: at_time,
        }
    }

    #[test]
    fn bonus_mission_shows_only_current_stage_with_fresh_progress() {
        let me = UserId::new();
        let other = Uuid::now_v7();
        let season = season();
        let matches = [
            paid_match(me.as_uuid(), other, 1, 12, 5_000_000),
            paid_match(me.as_uuid(), other, 2, 12, 5_000_000),
        ];
        let flags = flags();
        let first = assemble(AssembleInput {
            user_id: me,
            now: at(2, 15),
            season: Some(&season),
            registered_games: 4,
            matches: &matches,
            claims: &[],
            flags: &flags,
            extras: Extras::default(),
        });
        assert!(
            first
                .quests
                .iter()
                .all(|q| q.category != crate::quests::period::PeriodKind::PaidLadder)
        );
        let bonus = first.bonus_mission.as_ref().expect("stage 1 visible");
        assert_eq!(bonus.quest.id, "paid.volume:5");
        assert_eq!(bonus.dollars, 5);
        assert_eq!(bonus.quest.progress, 5_000_000);
        assert_eq!(bonus.quest.state, QuestState::Claimable);

        let weekly_paid = first
            .quests
            .iter()
            .find(|q| q.id == "weekly.paid-8")
            .expect("weekly paid stays independent");
        assert_eq!(weekly_paid.progress, 8_000_000);
        assert_eq!(weekly_paid.state, QuestState::Claimable);

        let claims = [claim_row(
            "paid.volume:5",
            &bonus.quest.period_id,
            at(2, 13),
            80,
        )];
        let second = assemble(AssembleInput {
            user_id: me,
            now: at(2, 15),
            season: Some(&season),
            registered_games: 4,
            matches: &matches,
            claims: &claims,
            flags: &flags,
            extras: Extras::default(),
        });
        let next = second.bonus_mission.as_ref().expect("stage 2 visible");
        assert_eq!(next.quest.id, "paid.volume:10");
        assert_eq!(next.dollars, 10);
        assert_eq!(next.quest.progress, 0);
        assert_eq!(next.quest.state, QuestState::Active);
        assert_eq!(second.season_quest_points, 80);
        let weekly_after = second
            .quests
            .iter()
            .find(|q| q.id == "weekly.paid-8")
            .unwrap();
        assert_eq!(weekly_after.progress, 8_000_000);
    }
}
