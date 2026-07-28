use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use sw_domain::SeasonId;

use crate::data::seasons::{PgSeasonRepo, SeasonRepo};
use crate::error::{AppError, AppResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_seasons))
        .route("/current", get(current_season))
        .route("/{season_id}", get(get_season))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListSeasonsQuery {
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

async fn list_seasons(
    State(state): State<AppState>,
    Query(query): Query<ListSeasonsQuery>,
) -> AppResult<Json<Vec<sw_domain::Season>>> {
    let (limit, offset) = clamp_page(query.limit, query.offset);
    let seasons = PgSeasonRepo::new(state.db.clone())
        .list(limit, offset)
        .await?;
    Ok(Json(seasons))
}

async fn current_season(State(state): State<AppState>) -> AppResult<Json<sw_domain::Season>> {
    let season = PgSeasonRepo::new(state.db.clone())
        .current()
        .await?
        .ok_or(AppError::NotFound("no active season"))?;
    Ok(Json(season))
}

async fn get_season(
    State(state): State<AppState>,
    Path(season_id): Path<i32>,
) -> AppResult<Json<sw_domain::Season>> {
    let season = PgSeasonRepo::new(state.db.clone())
        .get(SeasonId(season_id))
        .await?
        .ok_or(AppError::NotFound("season not found"))?;
    Ok(Json(season))
}
