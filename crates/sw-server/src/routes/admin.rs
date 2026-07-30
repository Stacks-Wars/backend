use axum::extract::{Path, State};
use axum::routing::{post, put};
use axum::{Json, Router};
use serde::Deserialize;
use sw_domain::SeasonId;

use crate::auth::AuthUser;
use crate::data::seasons::{PgSeasonRepo, SeasonRepo, UpdateSeasonInput};
use crate::error::AppResult;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/seasons", post(create_season))
        .route("/seasons/{season_id}", put(update_season))
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

/// Create the next quarterly season. Dates are computed server-side.
async fn create_season(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<CreateSeasonBody>,
) -> AppResult<Json<sw_domain::Season>> {
    auth.require_admin(&state.config.admin_emails)?;

    let season = PgSeasonRepo::new(state.db.clone())
        .create_next_quarter(body.name, body.description)
        .await?;

    Ok(Json(season))
}

/// Update season name / description only (dates stay fixed).
async fn update_season(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(season_id): Path<i32>,
    Json(body): Json<UpdateSeasonBody>,
) -> AppResult<Json<sw_domain::Season>> {
    auth.require_admin(&state.config.admin_emails)?;

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
