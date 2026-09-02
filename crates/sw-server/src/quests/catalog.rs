//! In-code quest catalog. Claims store `quest_id`, `catalog_version`, and the
//! reward actually granted. Adding a quest is appending a [`QuestDef`].

use super::period::PeriodKind;
use sw_domain::USDCX_MICROS_PER_UNIT;

pub const VERSION: i32 = 2;

pub const DAILY_NEW_OPPONENTS: &str = "daily.new-opponents-3";
pub const WEEKLY_REFERRAL: &str = "weekly.referral-1";
pub const MONTHLY_REFERRAL: &str = "monthly.referral-3";

/// Bonus mission stages: (dollars, reward points). Sequential, independent
/// progress per stage. Claiming one unlocks the next at zero.
pub const PAID_STAGES: &[(i64, i32)] = &[(5, 80), (10, 120), (20, 200), (50, 400), (100, 700)];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Metric {
    UsernameSet,
    HostedGames,
    JoinedGames,
    GamesPlayed,
    GamesWon,
    UniqueGames,
    UniqueOpponents,
    /// Distinct opponents in the period who were not faced in the prior 7 days.
    NewOpponents,
    PaidGames,
    PaidEntryMicro,
    ActiveDays,
    DailyClaims,
    AnyClaims,
    SuccessfulReferrals,
    LongestStreak,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Fixed(i64),
    /// Distinct games equal to the live registry size.
    RegisteredGames,
}

impl Target {
    pub fn value(self, registered_games: usize) -> i64 {
        match self {
            Self::Fixed(n) => n,
            Self::RegisteredGames => registered_games.max(1) as i64,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cta {
    pub href: &'static str,
    pub label: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuestDef {
    pub id: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub category: PeriodKind,
    pub metric: Metric,
    pub target: Target,
    pub reward_points: i32,
    pub cta: Cta,
}

const PLAY: Cta = Cta {
    href: "/lobbies",
    label: "Play",
};
const GAMES: Cta = Cta {
    href: "/games",
    label: "Pick a game",
};
const SETTINGS: Cta = Cta {
    href: "/settings",
    label: "Choose username",
};
const QUESTS: Cta = Cta {
    href: "/quests",
    label: "View quests",
};

const QUESTS_STATIC: &[QuestDef] = &[
    QuestDef {
        id: "gs.username",
        title: "Choose your username",
        description: "This is how other players see you.",
        category: PeriodKind::GettingStarted,
        metric: Metric::UsernameSet,
        target: Target::Fixed(1),
        reward_points: 40,
        cta: SETTINGS,
    },
    QuestDef {
        id: "gs.host",
        title: "Host your first match",
        description: "Create a lobby and play it out.",
        category: PeriodKind::GettingStarted,
        metric: Metric::HostedGames,
        target: Target::Fixed(1),
        reward_points: 60,
        cta: PLAY,
    },
    QuestDef {
        id: "gs.join",
        title: "Join a match",
        description: "Sit down in a room you didn't host.",
        category: PeriodKind::GettingStarted,
        metric: Metric::JoinedGames,
        target: Target::Fixed(1),
        reward_points: 60,
        cta: PLAY,
    },
    QuestDef {
        id: "gs.win",
        title: "Win your first match",
        description: "Finish first in a real lobby.",
        category: PeriodKind::GettingStarted,
        metric: Metric::GamesWon,
        target: Target::Fixed(1),
        reward_points: 80,
        cta: PLAY,
    },
    QuestDef {
        id: "daily.play-2",
        title: "Play 2 matches",
        description: "Jump into two lobbies today.",
        category: PeriodKind::Daily,
        metric: Metric::GamesPlayed,
        target: Target::Fixed(2),
        reward_points: 35,
        cta: PLAY,
    },
    QuestDef {
        id: "daily.games-2",
        title: "Try 2 different games today",
        description: "Don't stick to one title.",
        category: PeriodKind::Daily,
        metric: Metric::UniqueGames,
        target: Target::Fixed(2),
        reward_points: 45,
        cta: GAMES,
    },
    QuestDef {
        id: "daily.win-1",
        title: "Win a match today",
        description: "Finish first in any lobby.",
        category: PeriodKind::Daily,
        metric: Metric::GamesWon,
        target: Target::Fixed(1),
        reward_points: 50,
        cta: PLAY,
    },
    QuestDef {
        id: DAILY_NEW_OPPONENTS,
        title: "Meet 3 new opponents",
        description: "Play against 3 players you haven't faced in the last 7 days.",
        category: PeriodKind::Daily,
        metric: Metric::NewOpponents,
        target: Target::Fixed(3),
        reward_points: 55,
        cta: PLAY,
    },
    QuestDef {
        id: "daily.paid-1",
        title: "Play a paid match",
        description: "Put an entry fee on the line.",
        category: PeriodKind::Daily,
        metric: Metric::PaidGames,
        target: Target::Fixed(1),
        reward_points: 40,
        cta: PLAY,
    },
    QuestDef {
        id: "weekly.play-15",
        title: "Play 15 matches this week",
        description: "Keep showing up.",
        category: PeriodKind::Weekly,
        metric: Metric::GamesPlayed,
        target: Target::Fixed(15),
        reward_points: 180,
        cta: PLAY,
    },
    QuestDef {
        id: "weekly.win-5",
        title: "Win 5 matches",
        description: "Five wins this week.",
        category: PeriodKind::Weekly,
        metric: Metric::GamesWon,
        target: Target::Fixed(5),
        reward_points: 200,
        cta: PLAY,
    },
    QuestDef {
        id: "weekly.games-3",
        title: "Play 3 different games",
        description: "Switch titles at least twice this week.",
        category: PeriodKind::Weekly,
        metric: Metric::UniqueGames,
        target: Target::Fixed(3),
        reward_points: 150,
        cta: GAMES,
    },
    QuestDef {
        id: "weekly.opponents-8",
        title: "Play against 8 different players",
        description: "Get out of the same lobby circle.",
        category: PeriodKind::Weekly,
        metric: Metric::UniqueOpponents,
        target: Target::Fixed(8),
        reward_points: 180,
        cta: PLAY,
    },
    QuestDef {
        id: "weekly.days-5",
        title: "Keep your streak going",
        description: "Play on 5 different days this week.",
        category: PeriodKind::Weekly,
        metric: Metric::ActiveDays,
        target: Target::Fixed(5),
        reward_points: 220,
        cta: PLAY,
    },
    QuestDef {
        id: "weekly.paid-8",
        title: "Play $8 this week",
        description: "Paid entries this week, added up.",
        category: PeriodKind::Weekly,
        metric: Metric::PaidEntryMicro,
        target: Target::Fixed(8 * USDCX_MICROS_PER_UNIT),
        reward_points: 160,
        cta: PLAY,
    },
    QuestDef {
        id: "weekly.dailies-4",
        title: "Claim 4 daily quests",
        description: "Finish and claim four dailies this week.",
        category: PeriodKind::Weekly,
        metric: Metric::DailyClaims,
        target: Target::Fixed(4),
        reward_points: 140,
        cta: QUESTS,
    },
    QuestDef {
        id: WEEKLY_REFERRAL,
        title: "Bring a player in",
        description: "Someone you invited finishes Getting Started.",
        category: PeriodKind::Weekly,
        metric: Metric::SuccessfulReferrals,
        target: Target::Fixed(1),
        reward_points: 250,
        cta: QUESTS,
    },
    QuestDef {
        id: "monthly.play-40",
        title: "Play 40 matches this month",
        description: "A full month of play.",
        category: PeriodKind::Monthly,
        metric: Metric::GamesPlayed,
        target: Target::Fixed(40),
        reward_points: 500,
        cta: PLAY,
    },
    QuestDef {
        id: "monthly.win-12",
        title: "Win 12 matches",
        description: "Twelve wins this month.",
        category: PeriodKind::Monthly,
        metric: Metric::GamesWon,
        target: Target::Fixed(12),
        reward_points: 550,
        cta: PLAY,
    },
    QuestDef {
        id: "monthly.games-all",
        title: "Play every game",
        description: "One finished match in each title this month.",
        category: PeriodKind::Monthly,
        metric: Metric::UniqueGames,
        target: Target::RegisteredGames,
        reward_points: 400,
        cta: GAMES,
    },
    QuestDef {
        id: "monthly.opponents-15",
        title: "Play against 15 different players",
        description: "Widen the field this month.",
        category: PeriodKind::Monthly,
        metric: Metric::UniqueOpponents,
        target: Target::Fixed(15),
        reward_points: 500,
        cta: PLAY,
    },
    QuestDef {
        id: "monthly.days-12",
        title: "Play 12 days this month",
        description: "Don't disappear for weeks at a time.",
        category: PeriodKind::Monthly,
        metric: Metric::ActiveDays,
        target: Target::Fixed(12),
        reward_points: 600,
        cta: PLAY,
    },
    QuestDef {
        id: "monthly.paid-25",
        title: "Play $25 this month",
        description: "Paid entries this month, added up.",
        category: PeriodKind::Monthly,
        metric: Metric::PaidEntryMicro,
        target: Target::Fixed(25 * USDCX_MICROS_PER_UNIT),
        reward_points: 450,
        cta: PLAY,
    },
    QuestDef {
        id: "monthly.dailies-15",
        title: "Claim 15 daily quests",
        description: "Fifteen dailies claimed this month.",
        category: PeriodKind::Monthly,
        metric: Metric::DailyClaims,
        target: Target::Fixed(15),
        reward_points: 400,
        cta: QUESTS,
    },
    QuestDef {
        id: MONTHLY_REFERRAL,
        title: "Bring in 3 players",
        description: "Three people you invited finish Getting Started.",
        category: PeriodKind::Monthly,
        metric: Metric::SuccessfulReferrals,
        target: Target::Fixed(3),
        reward_points: 800,
        cta: QUESTS,
    },
    QuestDef {
        id: "seasonal.streak-10",
        title: "Keep a 10-day streak",
        description: "Play on 10 days in a row this season.",
        category: PeriodKind::Seasonal,
        metric: Metric::LongestStreak,
        target: Target::Fixed(10),
        reward_points: 400,
        cta: PLAY,
    },
    QuestDef {
        id: "seasonal.streak-30",
        title: "Keep a 30-day streak",
        description: "Play on 30 days in a row this season.",
        category: PeriodKind::Seasonal,
        metric: Metric::LongestStreak,
        target: Target::Fixed(30),
        reward_points: 1200,
        cta: PLAY,
    },
    QuestDef {
        id: "seasonal.streak-50",
        title: "Keep a 50-day streak",
        description: "Play on 50 days in a row this season.",
        category: PeriodKind::Seasonal,
        metric: Metric::LongestStreak,
        target: Target::Fixed(50),
        reward_points: 2000,
        cta: PLAY,
    },
    QuestDef {
        id: "seasonal.play-100",
        title: "Play 100 matches this season",
        description: "A hundred finished lobbies.",
        category: PeriodKind::Seasonal,
        metric: Metric::GamesPlayed,
        target: Target::Fixed(100),
        reward_points: 900,
        cta: PLAY,
    },
    QuestDef {
        id: "seasonal.win-30",
        title: "Win 30 matches this season",
        description: "Thirty wins on the board.",
        category: PeriodKind::Seasonal,
        metric: Metric::GamesWon,
        target: Target::Fixed(30),
        reward_points: 1000,
        cta: PLAY,
    },
    QuestDef {
        id: "seasonal.games-all",
        title: "Play every game",
        description: "One finished match in each title this season.",
        category: PeriodKind::Seasonal,
        metric: Metric::UniqueGames,
        target: Target::RegisteredGames,
        reward_points: 700,
        cta: GAMES,
    },
    QuestDef {
        id: "seasonal.opponents-40",
        title: "Play against 40 different players",
        description: "See the field this season.",
        category: PeriodKind::Seasonal,
        metric: Metric::UniqueOpponents,
        target: Target::Fixed(40),
        reward_points: 1000,
        cta: PLAY,
    },
    QuestDef {
        id: "seasonal.quests-40",
        title: "Claim 40 quests",
        description: "Forty claims this season, any kind.",
        category: PeriodKind::Seasonal,
        metric: Metric::AnyClaims,
        target: Target::Fixed(40),
        reward_points: 800,
        cta: QUESTS,
    },
];

pub fn paid_defs() -> Vec<QuestDef> {
    const TITLES: &[&str] = &["Play $5", "Play $10", "Play $20", "Play $50", "Play $100"];
    PAID_STAGES
        .iter()
        .enumerate()
        .map(|(i, &(dollars, reward))| QuestDef {
            id: paid_id_static(i),
            title: TITLES[i],
            description: if i + 1 == PAID_STAGES.len() {
                "The last bonus mission this season."
            } else {
                "Complete this bonus mission to unlock the next one."
            },
            category: PeriodKind::PaidLadder,
            metric: Metric::PaidEntryMicro,
            target: Target::Fixed(dollars * USDCX_MICROS_PER_UNIT),
            reward_points: reward,
            cta: PLAY,
        })
        .collect()
}

fn paid_id_static(index: usize) -> &'static str {
    match index {
        0 => "paid.volume:5",
        1 => "paid.volume:10",
        2 => "paid.volume:20",
        3 => "paid.volume:50",
        4 => "paid.volume:100",
        _ => unreachable!("paid ladder has five stages"),
    }
}

pub fn all_defs() -> Vec<QuestDef> {
    let mut out = QUESTS_STATIC.to_vec();
    out.extend(paid_defs());
    out
}

pub fn get(id: &str) -> Option<QuestDef> {
    all_defs().into_iter().find(|q| q.id == id)
}

pub fn paid_stage_index(quest_id: &str) -> Option<usize> {
    PAID_STAGES
        .iter()
        .enumerate()
        .find(|(i, _)| paid_id_static(*i) == quest_id)
        .map(|(i, _)| i)
}

pub fn previous_paid_id(index: usize) -> Option<&'static str> {
    if index == 0 {
        None
    } else {
        Some(paid_id_static(index - 1))
    }
}
