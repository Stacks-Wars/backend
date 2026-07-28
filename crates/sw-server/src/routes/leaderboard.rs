use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use sw_domain::{GameId, LeaderboardEntry, SeasonId};

use crate::data::seasons::{PgSeasonRepo, SeasonRepo};
use crate::data::stats::PgStatsRepo;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(leaderboard))
        .route("/seasons/{season_id}", get(season_leaderboard))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LeaderboardQuery {
    season_id: Option<i32>,
    game_id: Option<String>,
    #[serde(default = "default_limit")]
    limit: i64,
    #[serde(default)]
    offset: i64,
}

fn default_limit() -> i64 {
    50
}

fn clamp_page(limit: i64, offset: i64) -> (i64, i64) {
    let limit = limit.clamp(1, 100);
    let offset = offset.max(0);
    (limit, offset)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LeaderboardResponse {
    items: Vec<LeaderboardEntry>,
    total: i64,
    limit: i64,
    offset: i64,
}

async fn resolve_season_id(
    state: &AppState,
    season_id: Option<i32>,
) -> AppResult<SeasonId> {
    if let Some(id) = season_id {
        return Ok(SeasonId(id));
    }
    PgSeasonRepo::new(state.db.clone())
        .current()
        .await?
        .map(|s| s.id)
        .ok_or(AppError::NotFound("no active season"))
}

async fn fetch_leaderboard(
    state: &AppState,
    season_id: SeasonId,
    game_id: Option<String>,
    limit: i64,
    offset: i64,
) -> AppResult<LeaderboardResponse> {
    let (limit, offset) = clamp_page(limit, offset);
    let stats = PgStatsRepo::new(state.db.clone());

    let (items, total) = if let Some(raw) = game_id {
        let game_id = GameId::new(raw).map_err(|e| AppError::BadRequest(e.to_string()))?;
        stats
            .leaderboard_by_game(season_id, &game_id, limit, offset)
            .await?
    } else {
        stats
            .leaderboard_overall(season_id, limit, offset)
            .await?
    };

    Ok(LeaderboardResponse {
        items,
        total,
        limit,
        offset,
    })
}

async fn leaderboard(
    State(state): State<AppState>,
    Query(query): Query<LeaderboardQuery>,
) -> AppResult<Json<LeaderboardResponse>> {
    let season_id = resolve_season_id(&state, query.season_id).await?;
    let page = fetch_leaderboard(
        &state,
        season_id,
        query.game_id,
        query.limit,
        query.offset,
    )
    .await?;
    Ok(Json(page))
}

async fn season_leaderboard(
    State(state): State<AppState>,
    Path(season_id): Path<i32>,
    Query(query): Query<LeaderboardQuery>,
) -> AppResult<Json<LeaderboardResponse>> {
    let page = fetch_leaderboard(
        &state,
        SeasonId(season_id),
        query.game_id,
        query.limit,
        query.offset,
    )
    .await?;
    Ok(Json(page))
}
