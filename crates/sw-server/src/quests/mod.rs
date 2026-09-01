pub mod cache;
pub mod catalog;
pub mod evaluate;
pub mod ingest;
pub mod period;
pub mod streak;
pub mod view;

use chrono::Utc;
use redis::aio::ConnectionManager;
use sqlx::{PgPool, Postgres, Transaction};
use sw_domain::{Season, UserId};

use crate::data::quest_claims::{self, PgQuestRepo};
use crate::data::users::{PgUserRepo, QuestFlags};
use crate::error::AppResult;
use crate::quests::evaluate::{Extras, OpenPeriods};
use crate::quests::view::{AssembleInput, QuestMeResponse, assemble};

pub async fn load_me(
    db: &PgPool,
    redis: &mut ConnectionManager,
    user_id: UserId,
    season: Option<&Season>,
    registered_games: usize,
    use_cache: bool,
) -> AppResult<QuestMeResponse> {
    let now = Utc::now();
    let stamped = PgQuestRepo::new(db.clone())
        .maybe_stamp_getting_started(user_id)
        .await?;

    if use_cache
        && !stamped
        && let Some(raw) = cache::get_json(redis, user_id).await
        && let Ok(cached) = serde_json::from_str::<QuestMeResponse>(&raw)
    {
        return Ok(cached);
    }

    let snapshot = load_from_db(db, user_id, season, registered_games, now).await?;
    let ttl = period::cache_ttl_secs(now);
    if let Ok(raw) = serde_json::to_string(&snapshot) {
        cache::set_json(redis, user_id, &raw, ttl).await;
    }
    Ok(snapshot)
}

pub async fn load_from_db(
    db: &PgPool,
    user_id: UserId,
    season: Option<&Season>,
    registered_games: usize,
    now: chrono::DateTime<Utc>,
) -> AppResult<QuestMeResponse> {
    let periods = OpenPeriods::current(
        now,
        season.map(|s| s.id.as_i32()),
        season.map(|s| s.starts_at),
        season.map(|s| s.ends_at),
    );
    let since = periods.covering_start();
    let repo = PgQuestRepo::new(db.clone());
    let users = PgUserRepo::new(db.clone());

    let week_ids = periods.daily_ids_in_week();
    let month_ids = periods.daily_ids_in_month();
    let (matches, claims, flags, referrals, daily_week, daily_month, season_claims) = tokio::try_join!(
        repo.qualifying_matches(user_id, since),
        repo.claims_for_user(user_id),
        users.quest_flags(user_id),
        repo.successful_referral_count(user_id),
        repo.daily_claim_count(user_id, &week_ids),
        repo.daily_claim_count(user_id, &month_ids),
        async {
            match season.map(|s| s.id.as_i32()) {
                Some(id) => repo.season_claim_count(user_id, id).await,
                None => Ok(0),
            }
        },
    )?;

    let flags = flags.ok_or(crate::error::AppError::NotFound("user"))?;
    let gs = repo.getting_started_actions(user_id).await?;
    let extras = Extras {
        getting_started: gs,
        referral_successes: referrals,
        daily_claims_in_week: daily_week,
        daily_claims_in_month: daily_month,
        any_claims_in_season: season_claims,
        season_streak: Default::default(),
    };

    Ok(assemble(AssembleInput {
        user_id,
        now,
        season,
        registered_games,
        matches: &matches,
        claims: &claims,
        flags: &flags,
        extras,
    }))
}

pub async fn load_from_tx(
    tx: &mut Transaction<'_, Postgres>,
    user_id: UserId,
    season: Option<&Season>,
    registered_games: usize,
    now: chrono::DateTime<Utc>,
) -> AppResult<QuestMeResponse> {
    let periods = OpenPeriods::current(
        now,
        season.map(|s| s.id.as_i32()),
        season.map(|s| s.starts_at),
        season.map(|s| s.ends_at),
    );
    let since = periods.covering_start();
    let week_ids = periods.daily_ids_in_week();
    let month_ids = periods.daily_ids_in_month();

    let matches = quest_claims::qualifying_matches(&mut **tx, user_id, since).await?;
    let claims = quest_claims::claims_for_user(&mut **tx, user_id).await?;
    let flags = quest_flags_tx(tx, user_id).await?;
    let referrals = quest_claims::successful_referral_count(&mut **tx, user_id).await?;
    let daily_week = quest_claims::daily_claim_count(&mut **tx, user_id, &week_ids).await?;
    let daily_month = quest_claims::daily_claim_count(&mut **tx, user_id, &month_ids).await?;
    let season_claims = match season.map(|s| s.id.as_i32()) {
        Some(id) => quest_claims::season_claim_count(&mut **tx, user_id, id).await?,
        None => 0,
    };
    let gs = quest_claims::getting_started_actions(&mut **tx, user_id).await?;

    Ok(assemble(AssembleInput {
        user_id,
        now,
        season,
        registered_games,
        matches: &matches,
        claims: &claims,
        flags: &flags,
        extras: Extras {
            getting_started: gs,
            referral_successes: referrals,
            daily_claims_in_week: daily_week,
            daily_claims_in_month: daily_month,
            any_claims_in_season: season_claims,
            season_streak: Default::default(),
        },
    }))
}

async fn quest_flags_tx(
    tx: &mut Transaction<'_, Postgres>,
    user_id: UserId,
) -> AppResult<QuestFlags> {
    PgUserRepo::quest_flags_on(&mut **tx, user_id)
        .await?
        .ok_or(crate::error::AppError::NotFound("user"))
}
