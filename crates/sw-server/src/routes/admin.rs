use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use sw_domain::SeasonId;

use crate::data::seasons::{PgSeasonRepo, SeasonRepo, UpdateSeasonInput};
use crate::error::{AppError, AppResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/health-detail", get(health_detail))
        .route("/seasons", post(create_season))
        .route("/seasons/{season_id}", put(update_season))
        .route("/games/reload", post(reload_games))
}

fn require_internal_secret(headers: &HeaderMap, expected: &str) -> AppResult<()> {
    let provided = headers
        .get("x-internal-secret")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if provided.is_empty() || provided != expected {
        return Err(AppError::Unauthorized("invalid internal secret"));
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateSeasonBody {
    name: String,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateSeasonBody {
    name: String,
    #[serde(default)]
    description: Option<String>,
}

async fn health_detail() -> AppResult<()> {
    Err(AppError::NotImplemented("admin health detail"))
}

/// Create the next quarterly season. Dates are computed server-side.
async fn create_season(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CreateSeasonBody>,
) -> AppResult<Json<sw_domain::Season>> {
    require_internal_secret(&headers, &state.config.internal_api_secret)?;

    let season = PgSeasonRepo::new(state.db.clone())
        .create_next_quarter(body.name, body.description)
        .await?;

    Ok(Json(season))
}

/// Update season name / description only (dates stay fixed).
async fn update_season(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(season_id): Path<i32>,
    Json(body): Json<UpdateSeasonBody>,
) -> AppResult<Json<sw_domain::Season>> {
    require_internal_secret(&headers, &state.config.internal_api_secret)?;

    let season = PgSeasonRepo::new(state.db.clone())
        .update(
            SeasonId(season_id),
            UpdateSeasonInput {
                name: body.name,
                description: body.description,
            },
        )
        .await?;

    Ok(Json(season))
}

async fn reload_games() -> AppResult<()> {
    Err(AppError::NotImplemented("admin reload games"))
}
