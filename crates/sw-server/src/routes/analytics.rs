use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use serde::Deserialize;
use sw_domain::{ChainId, SeasonId};

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
    let chain = parse_chain_filter(query.chain.as_deref())?;

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

/// `all` and blanks mean "no chain filter". Anything else has to be a known
/// chain, normalised through the domain enum so aliases (`bot`, `stx`) resolve
/// to the lowercase ids stored in Postgres.
fn parse_chain_filter(raw: Option<&str>) -> AppResult<Option<String>> {
    let Some(value) = raw.map(str::trim).filter(|v| !v.is_empty() && *v != "all") else {
        return Ok(None);
    };
    value
        .parse::<ChainId>()
        .map(|chain| Some(chain.as_str().to_owned()))
        .map_err(AppError::BadRequest)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chain_filter_accepts_every_settlement_chain() {
        for id in ChainId::ALL {
            assert_eq!(
                parse_chain_filter(Some(id.as_str())).unwrap(),
                Some(id.as_str().to_owned()),
                "{id} should be filterable"
            );
        }
    }

    #[test]
    fn chain_filter_treats_blank_and_all_as_no_filter() {
        assert_eq!(parse_chain_filter(None).unwrap(), None);
        assert_eq!(parse_chain_filter(Some("")).unwrap(), None);
        assert_eq!(parse_chain_filter(Some("   ")).unwrap(), None);
        assert_eq!(parse_chain_filter(Some("all")).unwrap(), None);
    }

    #[test]
    fn chain_filter_normalises_case_and_aliases() {
        assert_eq!(
            parse_chain_filter(Some("BOT")).unwrap(),
            Some("botchain".into())
        );
        assert_eq!(
            parse_chain_filter(Some(" stx ")).unwrap(),
            Some("stacks".into())
        );
        assert_eq!(
            parse_chain_filter(Some("SOL")).unwrap(),
            Some("solana".into())
        );
        assert_eq!(
            parse_chain_filter(Some("arb")).unwrap(),
            Some("arbitrum".into())
        );
    }

    #[test]
    fn chain_filter_rejects_unknown_chain() {
        assert!(parse_chain_filter(Some("dogecoin")).is_err());
    }
}
