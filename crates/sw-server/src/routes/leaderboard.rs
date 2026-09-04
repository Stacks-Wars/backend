use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use sw_domain::{GameId, LeaderboardEntry, SeasonId};

use crate::data::quest_claims::PgQuestRepo;
use crate::data::stats::PgStatsRepo;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(leaderboard))
        .route("/seasons/{season_id}", get(season_leaderboard))
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum Board {
    #[default]
    Game,
    Quests,
    All,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LeaderboardQuery {
    season_id: Option<i32>,
    game_id: Option<String>,
    #[serde(default)]
    board: Board,
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

/// `None` means every season (the season dropdown's All option).
async fn fetch_leaderboard(
    state: &AppState,
    season_id: Option<SeasonId>,
    game_id: Option<String>,
    board: Board,
    limit: i64,
    offset: i64,
) -> AppResult<LeaderboardResponse> {
    let (limit, offset) = clamp_page(limit, offset);

    let (items, total) = match board {
        Board::Game => {
            let stats = PgStatsRepo::new(state.db.clone());
            if let Some(raw) = game_id {
                let game_id = GameId::new(raw).map_err(|e| AppError::BadRequest(e.to_string()))?;
                stats
                    .leaderboard_by_game(season_id, &game_id, limit, offset)
                    .await?
            } else {
                stats.leaderboard_overall(season_id, limit, offset).await?
            }
        }
        Board::Quests => {
            PgQuestRepo::new(state.db.clone())
                .leaderboard_quests(season_id, limit, offset)
                .await?
        }
        Board::All => {
            PgQuestRepo::new(state.db.clone())
                .leaderboard_all(season_id, limit, offset)
                .await?
        }
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
    let page = fetch_leaderboard(
        &state,
        query.season_id.map(SeasonId),
        query.game_id,
        query.board,
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
        Some(SeasonId(season_id)),
        query.game_id,
        query.board,
        query.limit,
        query.offset,
    )
    .await?;
    Ok(Json(page))
}
