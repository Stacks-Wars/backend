//! Platform KPI aggregates for the unlisted investor dashboard.
//!
//! Derived from existing tables. Qualifying matches match the quest engine:
//! `player_count >= 2`. Platform fee is the protocol 2% take on finished paid
//! matches that actually paid a winner — not dest/game fees, and not inferred
//! from unpaid volume.

use chrono::{DateTime, Duration, Utc};
use redis::AsyncCommands;
use redis::aio::ConnectionManager;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};
use tracing::warn;

use crate::error::{AppError, AppResult};

const PLATFORM_FEE_PCT: i64 = 2;
const CACHE_TTL_SECS: u64 = 90;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AnalyticsGrain {
    Day,
    Week,
    Month,
}

impl AnalyticsGrain {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Day => "day",
            Self::Week => "week",
            Self::Month => "month",
        }
    }

    pub fn for_span(from: DateTime<Utc>, to: DateTime<Utc>) -> Self {
        let days = (to - from).num_days().max(1);
        if days <= 90 {
            Self::Day
        } else if days <= 420 {
            Self::Week
        } else {
            Self::Month
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AnalyticsScope {
    Overall,
    Season,
    Custom,
}

#[derive(Debug, Clone)]
pub struct AnalyticsFilter {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    pub scope: AnalyticsScope,
    pub season_id: Option<i32>,
    pub game_id: Option<String>,
    pub chain: Option<String>,
}

impl AnalyticsFilter {
    pub fn activity_scoped(&self) -> bool {
        self.game_id.is_some() || self.chain.is_some()
    }

    pub fn cache_key(&self) -> String {
        format!(
            "sw:analytics:v1:{}:{}:{}:{}:{}:{}",
            self.from.timestamp(),
            self.to.timestamp(),
            self.season_id.map(|id| id.to_string()).unwrap_or_default(),
            self.game_id.as_deref().unwrap_or("-"),
            self.chain.as_deref().unwrap_or("-"),
            self.scope_label(),
        )
    }

    fn scope_label(&self) -> &'static str {
        match self.scope {
            AnalyticsScope::Overall => "overall",
            AnalyticsScope::Season => "season",
            AnalyticsScope::Custom => "custom",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsRange {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    pub grain: AnalyticsGrain,
    pub scope: AnalyticsScope,
    pub season_id: Option<i32>,
    pub game_id: Option<String>,
    pub chain: Option<String>,
    pub activity_scoped: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsKpis {
    pub total_users: i64,
    pub new_users: i64,
    pub getting_started_completed: i64,
    pub getting_started_completion_rate: Option<f64>,
    pub active_users: i64,
    pub returning_users: i64,
    pub games_played: i64,
    pub total_lobbies: i64,
    pub paid_lobbies_created: i64,
    pub paid_lobbies_completed: i64,
    pub total_volume_micro: i64,
    pub platform_fees_micro: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnboardingFunnel {
    pub signups: i64,
    pub started: i64,
    pub completed: i64,
    pub start_rate: Option<f64>,
    pub complete_rate: Option<f64>,
    pub complete_of_started_rate: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetentionSnapshot {
    pub active_users: i64,
    pub reactivated_users: i64,
    pub repeat_users: i64,
    pub users_with_play: i64,
    pub repeat_rate: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestAnalytics {
    pub claims: i64,
    pub unique_claimers: i64,
    pub points_awarded: i64,
    pub getting_started_claims: i64,
    pub daily_claims: i64,
    pub weekly_claims: i64,
    pub monthly_claims: i64,
    pub seasonal_claims: i64,
    pub paid_ladder_claims: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsPoint {
    pub bucket: DateTime<Utc>,
    pub new_users: i64,
    pub active_users: i64,
    pub returning_users: i64,
    pub games_played: i64,
    pub paid_lobbies_created: i64,
    pub paid_lobbies_completed: i64,
    pub volume_micro: i64,
    pub platform_fees_micro: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeeBreakdown {
    pub key: String,
    pub paid_matches: i64,
    pub volume_micro: i64,
    pub platform_fees_micro: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeasonComparisonRow {
    pub season_id: i32,
    pub name: String,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub new_users: i64,
    pub active_users: i64,
    pub games_played: i64,
    pub paid_lobbies_completed: i64,
    pub volume_micro: i64,
    pub platform_fees_micro: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsReport {
    pub range: AnalyticsRange,
    pub kpis: AnalyticsKpis,
    pub funnel: OnboardingFunnel,
    pub retention: RetentionSnapshot,
    pub quests: QuestAnalytics,
    pub series: Vec<AnalyticsPoint>,
    pub fees_by_chain: Vec<FeeBreakdown>,
    pub fees_by_game: Vec<FeeBreakdown>,
    pub season_comparison: Vec<SeasonComparisonRow>,
    pub definitions: AnalyticsDefinitions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsDefinitions {
    pub qualifying_match: String,
    pub platform_fee: String,
    pub volume: String,
    pub active_users: String,
    pub returning_users: String,
    pub getting_started: String,
}

fn definitions() -> AnalyticsDefinitions {
    AnalyticsDefinitions {
        qualifying_match: "A finished match with at least two players. Solo finishes are excluded, matching quest scoring.".into(),
        platform_fee: "Expected 2% protocol take on the pot of finished paid matches that paid a winner. Draws (refunded) and unpaid games are excluded. Dest/game developer fees are not platform revenue. This is the protocol split, not a confirmed on-chain receipt ledger.".into(),
        volume: "Sum of pots on finished paid qualifying matches in the window, including draws that were later refunded.".into(),
        active_users: "Distinct accounts that sat in a qualifying match that finished in the window.".into(),
        returning_users: "For a season or custom range: players active in the window whose first qualifying match was before it. For all-time: players with two or more distinct days of qualifying play.".into(),
        getting_started: "Completed means getting_started_completed_at is set (username + host + join + win). Started means the user opened quests, set a username, or finished Getting Started. Completion rate is completed / signups in the same cohort window.".into(),
    }
}

pub async fn earliest_event_at(pool: &PgPool) -> AppResult<DateTime<Utc>> {
    let row: (Option<DateTime<Utc>>,) = sqlx::query_as(
        r#"
        SELECT LEAST(
            (SELECT MIN(created_at) FROM users WHERE deleted_at IS NULL),
            (SELECT MIN(created_at) FROM lobbies),
            (SELECT MIN(finished_at) FROM matches),
            (SELECT MIN(claimed_at) FROM quest_claims)
        )
        "#,
    )
    .fetch_one(pool)
    .await
    .map_err(|err| AppError::Internal(err.into()))?;

    Ok(row.0.unwrap_or_else(|| Utc::now() - Duration::days(30)))
}

pub async fn load_report(pool: &PgPool, filter: &AnalyticsFilter) -> AppResult<AnalyticsReport> {
    let grain = AnalyticsGrain::for_span(filter.from, filter.to);
    let game = filter.game_id.as_deref();
    let chain = filter.chain.as_deref();

    let (kpis, series, quests, fees_chain, fees_game, seasons) = tokio::try_join!(
        load_kpis(pool, filter.from, filter.to, game, chain),
        load_series(pool, filter.from, filter.to, grain, game, chain),
        load_quests(pool, filter.from, filter.to),
        load_fee_breakdown(pool, filter.from, filter.to, game, chain, Breakdown::Chain),
        load_fee_breakdown(pool, filter.from, filter.to, game, chain, Breakdown::Game),
        load_season_comparison(pool, game, chain, filter.scope == AnalyticsScope::Season),
    )?;

    let returning_users = match filter.scope {
        AnalyticsScope::Overall => kpis.repeat_users,
        AnalyticsScope::Season | AnalyticsScope::Custom => kpis.reactivated_users,
    };

    let funnel = OnboardingFunnel {
        signups: kpis.new_users,
        started: kpis.funnel_started,
        completed: kpis.funnel_completed,
        start_rate: ratio(kpis.funnel_started, kpis.new_users),
        complete_rate: ratio(kpis.funnel_completed, kpis.new_users),
        complete_of_started_rate: ratio(kpis.funnel_completed, kpis.funnel_started),
    };

    Ok(AnalyticsReport {
        range: AnalyticsRange {
            from: filter.from,
            to: filter.to,
            grain,
            scope: filter.scope,
            season_id: filter.season_id,
            game_id: filter.game_id.clone(),
            chain: filter.chain.clone(),
            activity_scoped: filter.activity_scoped(),
        },
        kpis: AnalyticsKpis {
            total_users: kpis.total_users,
            new_users: kpis.new_users,
            getting_started_completed: kpis.gs_completed_in_range,
            getting_started_completion_rate: funnel.complete_rate,
            active_users: kpis.active_users,
            returning_users,
            games_played: kpis.games_played,
            total_lobbies: kpis.total_lobbies,
            paid_lobbies_created: kpis.paid_lobbies_created,
            paid_lobbies_completed: kpis.paid_lobbies_completed,
            total_volume_micro: kpis.total_volume_micro,
            platform_fees_micro: kpis.platform_fees_micro,
        },
        funnel,
        retention: RetentionSnapshot {
            active_users: kpis.active_users,
            reactivated_users: kpis.reactivated_users,
            repeat_users: kpis.repeat_users,
            users_with_play: kpis.users_with_play,
            repeat_rate: ratio(kpis.repeat_users, kpis.users_with_play),
        },
        quests,
        series,
        fees_by_chain: fees_chain,
        fees_by_game: fees_game,
        season_comparison: seasons,
        definitions: definitions(),
    })
}

fn ratio(num: i64, den: i64) -> Option<f64> {
    if den <= 0 {
        None
    } else {
        Some((num as f64) / (den as f64))
    }
}

#[derive(Debug, FromRow)]
struct KpiRow {
    total_users: i64,
    new_users: i64,
    gs_completed_in_range: i64,
    funnel_started: i64,
    funnel_completed: i64,
    games_played: i64,
    total_lobbies: i64,
    paid_lobbies_created: i64,
    paid_lobbies_completed: i64,
    total_volume_micro: i64,
    platform_fees_micro: i64,
    active_users: i64,
    reactivated_users: i64,
    repeat_users: i64,
    users_with_play: i64,
}

async fn load_kpis(
    pool: &PgPool,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    game_id: Option<&str>,
    chain: Option<&str>,
) -> AppResult<KpiRow> {
    sqlx::query_as::<_, KpiRow>(
        r#"
        WITH scoped AS (
            SELECT
                m.id,
                m.lobby_id,
                m.game_id,
                m.pot_micro,
                m.entry_amount_micro,
                m.finished_at,
                l.chain::text AS chain
            FROM matches m
            INNER JOIN lobbies l ON l.id = m.lobby_id
            WHERE m.player_count >= 2
              AND ($3::text IS NULL OR m.game_id = $3)
              AND ($4::text IS NULL OR l.chain::text = $4)
        ),
        window_matches AS (
            SELECT * FROM scoped
            WHERE finished_at >= $1 AND finished_at < $2
        ),
        first_play AS (
            SELECT mp.user_id, MIN(s.finished_at) AS first_at
            FROM match_players mp
            INNER JOIN scoped s ON s.id = mp.match_id
            GROUP BY mp.user_id
        ),
        window_players AS (
            SELECT DISTINCT mp.user_id
            FROM match_players mp
            INNER JOIN window_matches w ON w.id = mp.match_id
        ),
        repeat_players AS (
            SELECT mp.user_id
            FROM match_players mp
            INNER JOIN window_matches w ON w.id = mp.match_id
            GROUP BY mp.user_id
            HAVING COUNT(DISTINCT (w.finished_at AT TIME ZONE 'UTC')::date) >= 2
        ),
        fee_matches AS (
            SELECT w.pot_micro
            FROM window_matches w
            WHERE w.entry_amount_micro > 0
              AND w.pot_micro > 0
              AND EXISTS (
                  SELECT 1
                  FROM match_players mp
                  WHERE mp.match_id = w.id
                    AND mp.is_winner
                    AND mp.prize_micro > 0
              )
        ),
        users_alive AS (
            SELECT
                created_at,
                username,
                quest_intro_seen_at,
                getting_started_completed_at
            FROM users
            WHERE deleted_at IS NULL
        )
        SELECT
            (SELECT COUNT(*) FROM users_alive WHERE created_at < $2)::bigint AS total_users,
            (SELECT COUNT(*) FROM users_alive WHERE created_at >= $1 AND created_at < $2)::bigint AS new_users,
            (
                SELECT COUNT(*)
                FROM users_alive
                WHERE getting_started_completed_at >= $1
                  AND getting_started_completed_at < $2
            )::bigint AS gs_completed_in_range,
            (
                SELECT COUNT(*)
                FROM users_alive
                WHERE created_at >= $1 AND created_at < $2
                  AND (
                      quest_intro_seen_at IS NOT NULL
                      OR username IS NOT NULL
                      OR getting_started_completed_at IS NOT NULL
                  )
            )::bigint AS funnel_started,
            (
                SELECT COUNT(*)
                FROM users_alive
                WHERE created_at >= $1 AND created_at < $2
                  AND getting_started_completed_at IS NOT NULL
            )::bigint AS funnel_completed,
            (SELECT COUNT(*) FROM window_matches)::bigint AS games_played,
            (
                SELECT COUNT(*)
                FROM lobbies l
                WHERE l.created_at >= $1 AND l.created_at < $2
                  AND ($3::text IS NULL OR l.game_id = $3)
                  AND ($4::text IS NULL OR l.chain::text = $4)
            )::bigint AS total_lobbies,
            (
                SELECT COUNT(*)
                FROM lobbies l
                WHERE l.created_at >= $1 AND l.created_at < $2
                  AND l.entry_amount_micro > 0
                  AND ($3::text IS NULL OR l.game_id = $3)
                  AND ($4::text IS NULL OR l.chain::text = $4)
            )::bigint AS paid_lobbies_created,
            (
                SELECT COUNT(*)
                FROM window_matches
                WHERE entry_amount_micro > 0
            )::bigint AS paid_lobbies_completed,
            (
                SELECT COALESCE(SUM(pot_micro), 0)
                FROM window_matches
                WHERE entry_amount_micro > 0
            )::bigint AS total_volume_micro,
            (
                SELECT COALESCE(SUM((pot_micro * $5)::bigint / 100), 0)
                FROM fee_matches
            )::bigint AS platform_fees_micro,
            (SELECT COUNT(*) FROM window_players)::bigint AS active_users,
            (
                SELECT COUNT(*)
                FROM window_players wp
                INNER JOIN first_play fp ON fp.user_id = wp.user_id
                WHERE fp.first_at < $1
            )::bigint AS reactivated_users,
            (SELECT COUNT(*) FROM repeat_players)::bigint AS repeat_users,
            (SELECT COUNT(*) FROM window_players)::bigint AS users_with_play
        "#,
    )
    .bind(from)
    .bind(to)
    .bind(game_id)
    .bind(chain)
    .bind(PLATFORM_FEE_PCT)
    .fetch_one(pool)
    .await
    .map_err(|err| AppError::Internal(err.into()))
}

#[derive(Debug, FromRow)]
struct SeriesRow {
    bucket: DateTime<Utc>,
    new_users: i64,
    active_users: i64,
    returning_users: i64,
    games_played: i64,
    paid_lobbies_created: i64,
    paid_lobbies_completed: i64,
    volume_micro: i64,
    platform_fees_micro: i64,
}

async fn load_series(
    pool: &PgPool,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    grain: AnalyticsGrain,
    game_id: Option<&str>,
    chain: Option<&str>,
) -> AppResult<Vec<AnalyticsPoint>> {
    let rows = sqlx::query_as::<_, SeriesRow>(
        r#"
        WITH buckets AS (
            SELECT generate_series(
                date_trunc($5, $1),
                date_trunc($5, $2 - interval '1 microsecond'),
                CASE $5
                    WHEN 'week' THEN interval '1 week'
                    WHEN 'month' THEN interval '1 month'
                    ELSE interval '1 day'
                END
            ) AS bucket
        ),
        scoped AS (
            SELECT
                m.id,
                m.pot_micro,
                m.entry_amount_micro,
                m.finished_at,
                l.chain::text AS chain,
                l.game_id,
                l.created_at AS lobby_created_at
            FROM matches m
            INNER JOIN lobbies l ON l.id = m.lobby_id
            WHERE m.player_count >= 2
              AND ($3::text IS NULL OR m.game_id = $3)
              AND ($4::text IS NULL OR l.chain::text = $4)
        ),
        first_play AS (
            SELECT mp.user_id, MIN(s.finished_at) AS first_at
            FROM match_players mp
            INNER JOIN scoped s ON s.id = mp.match_id
            GROUP BY mp.user_id
        ),
        new_users AS (
            SELECT date_trunc($5, created_at) AS bucket, COUNT(*)::bigint AS n
            FROM users
            WHERE deleted_at IS NULL
              AND created_at >= $1 AND created_at < $2
            GROUP BY 1
        ),
        games AS (
            SELECT date_trunc($5, finished_at) AS bucket,
                   COUNT(*)::bigint AS n,
                   COUNT(*) FILTER (WHERE entry_amount_micro > 0)::bigint AS paid,
                   COALESCE(SUM(pot_micro) FILTER (WHERE entry_amount_micro > 0), 0)::bigint AS volume
            FROM scoped
            WHERE finished_at >= $1 AND finished_at < $2
            GROUP BY 1
        ),
        fees AS (
            SELECT date_trunc($5, s.finished_at) AS bucket,
                   COALESCE(SUM((s.pot_micro * $6)::bigint / 100), 0)::bigint AS fees
            FROM scoped s
            WHERE s.finished_at >= $1 AND s.finished_at < $2
              AND s.entry_amount_micro > 0
              AND s.pot_micro > 0
              AND EXISTS (
                  SELECT 1
                  FROM match_players mp
                  WHERE mp.match_id = s.id
                    AND mp.is_winner
                    AND mp.prize_micro > 0
              )
            GROUP BY 1
        ),
        paid_created AS (
            SELECT date_trunc($5, created_at) AS bucket, COUNT(*)::bigint AS n
            FROM lobbies
            WHERE created_at >= $1 AND created_at < $2
              AND entry_amount_micro > 0
              AND ($3::text IS NULL OR game_id = $3)
              AND ($4::text IS NULL OR chain::text = $4)
            GROUP BY 1
        ),
        bucket_players AS (
            SELECT date_trunc($5, s.finished_at) AS bucket, mp.user_id
            FROM scoped s
            INNER JOIN match_players mp ON mp.match_id = s.id
            WHERE s.finished_at >= $1 AND s.finished_at < $2
            GROUP BY 1, 2
        ),
        activity AS (
            SELECT
                bp.bucket,
                COUNT(*)::bigint AS active_users,
                COUNT(*) FILTER (WHERE fp.first_at < bp.bucket)::bigint AS returning_users
            FROM bucket_players bp
            INNER JOIN first_play fp ON fp.user_id = bp.user_id
            GROUP BY bp.bucket
        )
        SELECT
            b.bucket,
            COALESCE(nu.n, 0)::bigint AS new_users,
            COALESCE(a.active_users, 0)::bigint AS active_users,
            COALESCE(a.returning_users, 0)::bigint AS returning_users,
            COALESCE(g.n, 0)::bigint AS games_played,
            COALESCE(pc.n, 0)::bigint AS paid_lobbies_created,
            COALESCE(g.paid, 0)::bigint AS paid_lobbies_completed,
            COALESCE(g.volume, 0)::bigint AS volume_micro,
            COALESCE(f.fees, 0)::bigint AS platform_fees_micro
        FROM buckets b
        LEFT JOIN new_users nu ON nu.bucket = b.bucket
        LEFT JOIN games g ON g.bucket = b.bucket
        LEFT JOIN fees f ON f.bucket = b.bucket
        LEFT JOIN paid_created pc ON pc.bucket = b.bucket
        LEFT JOIN activity a ON a.bucket = b.bucket
        ORDER BY b.bucket
        "#,
    )
    .bind(from)
    .bind(to)
    .bind(game_id)
    .bind(chain)
    .bind(grain.as_str())
    .bind(PLATFORM_FEE_PCT)
    .fetch_all(pool)
    .await
    .map_err(|err| AppError::Internal(err.into()))?;

    Ok(rows
        .into_iter()
        .map(|row| AnalyticsPoint {
            bucket: row.bucket,
            new_users: row.new_users,
            active_users: row.active_users,
            returning_users: row.returning_users,
            games_played: row.games_played,
            paid_lobbies_created: row.paid_lobbies_created,
            paid_lobbies_completed: row.paid_lobbies_completed,
            volume_micro: row.volume_micro,
            platform_fees_micro: row.platform_fees_micro,
        })
        .collect())
}

#[derive(Debug, FromRow)]
struct QuestRow {
    claims: i64,
    unique_claimers: i64,
    points_awarded: i64,
    getting_started_claims: i64,
    daily_claims: i64,
    weekly_claims: i64,
    monthly_claims: i64,
    seasonal_claims: i64,
    paid_ladder_claims: i64,
}

async fn load_quests(
    pool: &PgPool,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> AppResult<QuestAnalytics> {
    let row = sqlx::query_as::<_, QuestRow>(
        r#"
        SELECT
            COUNT(*)::bigint AS claims,
            COUNT(DISTINCT user_id)::bigint AS unique_claimers,
            COALESCE(SUM(reward_points), 0)::bigint AS points_awarded,
            COUNT(*) FILTER (WHERE period_kind = 'getting_started')::bigint AS getting_started_claims,
            COUNT(*) FILTER (WHERE period_kind = 'daily')::bigint AS daily_claims,
            COUNT(*) FILTER (WHERE period_kind = 'weekly')::bigint AS weekly_claims,
            COUNT(*) FILTER (WHERE period_kind = 'monthly')::bigint AS monthly_claims,
            COUNT(*) FILTER (WHERE period_kind = 'seasonal')::bigint AS seasonal_claims,
            COUNT(*) FILTER (WHERE period_kind = 'paid_ladder')::bigint AS paid_ladder_claims
        FROM quest_claims
        WHERE claimed_at >= $1 AND claimed_at < $2
        "#,
    )
    .bind(from)
    .bind(to)
    .fetch_one(pool)
    .await
    .map_err(|err| AppError::Internal(err.into()))?;

    Ok(QuestAnalytics {
        claims: row.claims,
        unique_claimers: row.unique_claimers,
        points_awarded: row.points_awarded,
        getting_started_claims: row.getting_started_claims,
        daily_claims: row.daily_claims,
        weekly_claims: row.weekly_claims,
        monthly_claims: row.monthly_claims,
        seasonal_claims: row.seasonal_claims,
        paid_ladder_claims: row.paid_ladder_claims,
    })
}

#[derive(Debug, Clone, Copy)]
enum Breakdown {
    Chain,
    Game,
}

#[derive(Debug, FromRow)]
struct BreakdownRow {
    key: String,
    paid_matches: i64,
    volume_micro: i64,
    platform_fees_micro: i64,
}

async fn load_fee_breakdown(
    pool: &PgPool,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    game_id: Option<&str>,
    chain: Option<&str>,
    by: Breakdown,
) -> AppResult<Vec<FeeBreakdown>> {
    const BY_CHAIN: &str = r#"
        SELECT
            l.chain::text AS key,
            COUNT(*)::bigint AS paid_matches,
            COALESCE(SUM(m.pot_micro), 0)::bigint AS volume_micro,
            COALESCE(SUM((m.pot_micro * $5)::bigint / 100), 0)::bigint AS platform_fees_micro
        FROM matches m
        INNER JOIN lobbies l ON l.id = m.lobby_id
        WHERE m.player_count >= 2
          AND m.finished_at >= $1 AND m.finished_at < $2
          AND m.entry_amount_micro > 0
          AND m.pot_micro > 0
          AND ($3::text IS NULL OR m.game_id = $3)
          AND ($4::text IS NULL OR l.chain::text = $4)
          AND EXISTS (
              SELECT 1
              FROM match_players mp
              WHERE mp.match_id = m.id
                AND mp.is_winner
                AND mp.prize_micro > 0
          )
        GROUP BY 1
        ORDER BY platform_fees_micro DESC, key
        "#;
    const BY_GAME: &str = r#"
        SELECT
            m.game_id AS key,
            COUNT(*)::bigint AS paid_matches,
            COALESCE(SUM(m.pot_micro), 0)::bigint AS volume_micro,
            COALESCE(SUM((m.pot_micro * $5)::bigint / 100), 0)::bigint AS platform_fees_micro
        FROM matches m
        INNER JOIN lobbies l ON l.id = m.lobby_id
        WHERE m.player_count >= 2
          AND m.finished_at >= $1 AND m.finished_at < $2
          AND m.entry_amount_micro > 0
          AND m.pot_micro > 0
          AND ($3::text IS NULL OR m.game_id = $3)
          AND ($4::text IS NULL OR l.chain::text = $4)
          AND EXISTS (
              SELECT 1
              FROM match_players mp
              WHERE mp.match_id = m.id
                AND mp.is_winner
                AND mp.prize_micro > 0
          )
        GROUP BY 1
        ORDER BY platform_fees_micro DESC, key
        "#;

    let sql = match by {
        Breakdown::Chain => BY_CHAIN,
        Breakdown::Game => BY_GAME,
    };

    let rows = sqlx::query_as::<_, BreakdownRow>(sql)
        .bind(from)
        .bind(to)
        .bind(game_id)
        .bind(chain)
        .bind(PLATFORM_FEE_PCT)
        .fetch_all(pool)
        .await
        .map_err(|err| AppError::Internal(err.into()))?;

    Ok(rows
        .into_iter()
        .map(|row| FeeBreakdown {
            key: row.key,
            paid_matches: row.paid_matches,
            volume_micro: row.volume_micro,
            platform_fees_micro: row.platform_fees_micro,
        })
        .collect())
}

#[derive(Debug, FromRow)]
struct SeasonRow {
    season_id: i32,
    name: String,
    starts_at: DateTime<Utc>,
    ends_at: DateTime<Utc>,
    new_users: i64,
    active_users: i64,
    games_played: i64,
    paid_lobbies_completed: i64,
    volume_micro: i64,
    platform_fees_micro: i64,
}

async fn load_season_comparison(
    pool: &PgPool,
    game_id: Option<&str>,
    chain: Option<&str>,
    skip: bool,
) -> AppResult<Vec<SeasonComparisonRow>> {
    if skip {
        return Ok(Vec::new());
    }

    let rows = sqlx::query_as::<_, SeasonRow>(
        r#"
        SELECT
            s.id AS season_id,
            s.name,
            s.starts_at,
            s.ends_at,
            (
                SELECT COUNT(*)
                FROM users u
                WHERE u.deleted_at IS NULL
                  AND u.created_at >= s.starts_at
                  AND u.created_at <= s.ends_at
            )::bigint AS new_users,
            (
                SELECT COUNT(DISTINCT mp.user_id)
                FROM matches m
                INNER JOIN lobbies l ON l.id = m.lobby_id
                INNER JOIN match_players mp ON mp.match_id = m.id
                WHERE m.player_count >= 2
                  AND m.season_id = s.id
                  AND ($1::text IS NULL OR m.game_id = $1)
                  AND ($2::text IS NULL OR l.chain::text = $2)
            )::bigint AS active_users,
            (
                SELECT COUNT(*)
                FROM matches m
                INNER JOIN lobbies l ON l.id = m.lobby_id
                WHERE m.player_count >= 2
                  AND m.season_id = s.id
                  AND ($1::text IS NULL OR m.game_id = $1)
                  AND ($2::text IS NULL OR l.chain::text = $2)
            )::bigint AS games_played,
            (
                SELECT COUNT(*)
                FROM matches m
                INNER JOIN lobbies l ON l.id = m.lobby_id
                WHERE m.player_count >= 2
                  AND m.season_id = s.id
                  AND m.entry_amount_micro > 0
                  AND ($1::text IS NULL OR m.game_id = $1)
                  AND ($2::text IS NULL OR l.chain::text = $2)
            )::bigint AS paid_lobbies_completed,
            (
                SELECT COALESCE(SUM(m.pot_micro), 0)
                FROM matches m
                INNER JOIN lobbies l ON l.id = m.lobby_id
                WHERE m.player_count >= 2
                  AND m.season_id = s.id
                  AND m.entry_amount_micro > 0
                  AND ($1::text IS NULL OR m.game_id = $1)
                  AND ($2::text IS NULL OR l.chain::text = $2)
            )::bigint AS volume_micro,
            (
                SELECT COALESCE(SUM((m.pot_micro * $3)::bigint / 100), 0)
                FROM matches m
                INNER JOIN lobbies l ON l.id = m.lobby_id
                WHERE m.player_count >= 2
                  AND m.season_id = s.id
                  AND m.entry_amount_micro > 0
                  AND m.pot_micro > 0
                  AND ($1::text IS NULL OR m.game_id = $1)
                  AND ($2::text IS NULL OR l.chain::text = $2)
                  AND EXISTS (
                      SELECT 1
                      FROM match_players mp
                      WHERE mp.match_id = m.id
                        AND mp.is_winner
                        AND mp.prize_micro > 0
                  )
            )::bigint AS platform_fees_micro
        FROM seasons s
        ORDER BY s.starts_at ASC
        "#,
    )
    .bind(game_id)
    .bind(chain)
    .bind(PLATFORM_FEE_PCT)
    .fetch_all(pool)
    .await
    .map_err(|err| AppError::Internal(err.into()))?;

    Ok(rows
        .into_iter()
        .map(|row| SeasonComparisonRow {
            season_id: row.season_id,
            name: row.name,
            starts_at: row.starts_at,
            ends_at: row.ends_at,
            new_users: row.new_users,
            active_users: row.active_users,
            games_played: row.games_played,
            paid_lobbies_completed: row.paid_lobbies_completed,
            volume_micro: row.volume_micro,
            platform_fees_micro: row.platform_fees_micro,
        })
        .collect())
}

pub async fn cache_get(redis: &mut ConnectionManager, key: &str) -> Option<AnalyticsReport> {
    let raw: Option<String> = match redis.get(key).await {
        Ok(value) => value,
        Err(err) => {
            warn!(error = %err, "analytics cache get failed");
            return None;
        }
    };
    raw.and_then(|body| serde_json::from_str(&body).ok())
}

pub async fn cache_set(redis: &mut ConnectionManager, key: &str, report: &AnalyticsReport) {
    let Ok(body) = serde_json::to_string(report) else {
        return;
    };
    if let Err(err) = redis.set_ex::<_, _, ()>(key, body, CACHE_TTL_SECS).await {
        warn!(error = %err, "analytics cache set failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn grain_picks_day_week_month() {
        let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        assert_eq!(
            AnalyticsGrain::for_span(from, from + Duration::days(30)),
            AnalyticsGrain::Day
        );
        assert_eq!(
            AnalyticsGrain::for_span(from, from + Duration::days(120)),
            AnalyticsGrain::Week
        );
        assert_eq!(
            AnalyticsGrain::for_span(from, from + Duration::days(500)),
            AnalyticsGrain::Month
        );
    }

    #[test]
    fn ratio_none_on_zero_den() {
        assert_eq!(ratio(4, 0), None);
        assert_eq!(ratio(1, 4), Some(0.25));
    }
}
