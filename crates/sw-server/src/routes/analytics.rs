use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use serde::Deserialize;
use sw_domain::SeasonId;

use crate::data::analytics::{
    AnalyticsFilter, AnalyticsReport, AnalyticsScope, cache_get, cache_set, earliest_event_at,
    load_report,
};
use crate::data::seasons::{PgSeasonRepo, SeasonRepo};
use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsQuery {
    season_id: Option<i32>,
    from: Option<String>,
    to: Option<String>,
    game_id: Option<String>,
    chain: Option<String>,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/analytics", get(platform_analytics))
}

async fn platform_analytics(
    State(state): State<AppState>,
    Query(query): Query<AnalyticsQuery>,
) -> AppResult<Json<AnalyticsReport>> {
    let filter = resolve_filter(&state, query).await?;
    let cache_key = filter.cache_key();

    let mut redis = state.redis.clone();
    if let Some(hit) = cache_get(&mut redis, &cache_key).await {
        return Ok(Json(hit));
    }

    let report = load_report(&state.db, &filter).await?;
    cache_set(&mut redis, &cache_key, &report).await;
    Ok(Json(report))
}

async fn resolve_filter(state: &AppState, query: AnalyticsQuery) -> AppResult<AnalyticsFilter> {
    let game_id = query
        .game_id
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty() && *v != "all")
        .map(ToOwned::to_owned);
    let chain = query
        .chain
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty() && *v != "all")
        .map(|v| v.to_ascii_lowercase());
    if let Some(ref chain) = chain
        && chain != "solana"
        && chain != "stacks"
    {
        return Err(AppError::BadRequest("unknown chain".into()));
    }

    let now = Utc::now();

    if let Some(season_id) = query.season_id {
        let season = PgSeasonRepo::new(state.db.clone())
            .get(SeasonId(season_id))
            .await?
            .ok_or(AppError::NotFound("season not found"))?;
        let from = season.starts_at;
        let end = season.ends_at + Duration::microseconds(1);
        let to = if now < from {
            end
        } else {
            end.min(now).max(from + Duration::seconds(1))
        };
        return Ok(AnalyticsFilter {
            from,
            to,
            scope: AnalyticsScope::Season,
            season_id: Some(season_id),
            game_id,
            chain,
        });
    }

    match (query.from.as_deref(), query.to.as_deref()) {
        (None, None) => {
            let from = earliest_event_at(&state.db).await?;
            Ok(AnalyticsFilter {
                from,
                to: now,
                scope: AnalyticsScope::Overall,
                season_id: None,
                game_id,
                chain,
            })
        }
        (Some(from_raw), to_raw) => {
            let from = parse_bound(from_raw, false)?;
            let mut to = match to_raw {
                Some(raw) => parse_bound(raw, true)?,
                None => now,
            };
            to = to.min(now + Duration::days(1));
            if to <= from {
                return Err(AppError::BadRequest("to must be after from".into()));
            }
            if to - from > Duration::days(366 * 10) {
                return Err(AppError::BadRequest("range is too large".into()));
            }
            Ok(AnalyticsFilter {
                from,
                to,
                scope: AnalyticsScope::Custom,
                season_id: None,
                game_id,
                chain,
            })
        }
        (None, Some(_)) => Err(AppError::BadRequest(
            "from is required when to is set".into(),
        )),
    }
}

/// Date-only values are UTC midnights. `end` makes a date inclusive by advancing
/// one day so the query window stays half-open.
fn parse_bound(raw: &str, end: bool) -> AppResult<DateTime<Utc>> {
    let trimmed = raw.trim();
    if trimmed.len() == 10 {
        let date = NaiveDate::parse_from_str(trimmed, "%Y-%m-%d")
            .map_err(|_| AppError::BadRequest("invalid date".into()))?;
        let mut midnight = Utc.from_utc_datetime(
            &date
                .and_hms_opt(0, 0, 0)
                .ok_or(AppError::BadRequest("invalid date".into()))?,
        );
        if end {
            midnight += Duration::days(1);
        }
        return Ok(midnight);
    }

    DateTime::parse_from_rfc3339(trimmed)
        .map(|dt| dt.with_timezone(&Utc))
        .or_else(|_| {
            trimmed
                .parse::<DateTime<Utc>>()
                .map_err(|_| AppError::BadRequest("invalid timestamp".into()))
        })
}
