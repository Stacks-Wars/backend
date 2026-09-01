use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::data::quest_claims::{NewQuestClaim, PgQuestRepo, QuestClaimRow};
use crate::data::seasons::{PgSeasonRepo, SeasonRepo};
use crate::error::{AppError, AppResult};
use crate::quests::catalog;
use crate::quests::evaluate::QuestState;
use crate::quests::view::{QuestMeResponse, is_claimable, paid_period_id, quest_view_for};
use crate::quests::{self, cache};
use crate::services::realtime;
use crate::state::AppState;

pub fn read_router() -> Router<AppState> {
    Router::new().route("/me", get(get_me))
}

pub fn write_router() -> Router<AppState> {
    Router::new().route("/claims", post(claim))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClaimBody {
    quest_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ClaimView {
    id: Uuid,
    quest_id: String,
    period_id: String,
    reward_points: i32,
    claimed_at: chrono::DateTime<Utc>,
    already_claimed: bool,
}

impl ClaimView {
    fn from_row(row: QuestClaimRow, already_claimed: bool) -> Self {
        Self {
            id: row.id,
            quest_id: row.quest_id,
            period_id: row.period_id,
            reward_points: row.reward_points,
            claimed_at: row.claimed_at,
            already_claimed,
        }
    }
}

async fn get_me(State(state): State<AppState>, auth: AuthUser) -> AppResult<Json<QuestMeResponse>> {
    let season = PgSeasonRepo::new(state.db.clone()).current().await?;
    let mut redis = state.redis.clone();
    let snapshot = quests::load_me(
        &state.db,
        &mut redis,
        auth.user_id,
        season.as_ref(),
        state.games.len(),
        true,
    )
    .await?;
    Ok(Json(snapshot))
}

async fn claim(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<ClaimBody>,
) -> AppResult<Json<ClaimView>> {
    let quest_id = body.quest_id.trim();
    if quest_id.is_empty() {
        return Err(AppError::BadRequest("questId is required".into()));
    }
    let def = catalog::get(quest_id).ok_or(AppError::NotFound("quest"))?;
    let season = PgSeasonRepo::new(state.db.clone()).current().await?;
    let now = Utc::now();

    let mut tx = state
        .db
        .begin()
        .await
        .map_err(|err| AppError::Internal(err.into()))?;

    let snapshot = quests::load_from_tx(
        &mut tx,
        auth.user_id,
        season.as_ref(),
        state.games.len(),
        now,
    )
    .await?;

    let view = match quest_view_for(&snapshot, def.id) {
        Some(view) => view,
        None if catalog::paid_stage_index(def.id).is_some() => {
            let period_id = paid_period_id(&snapshot);
            if let Some(period_id) = period_id
                && let Some(existing) =
                    PgQuestRepo::get_claim(&mut tx, auth.user_id, def.id, period_id).await?
            {
                tx.commit()
                    .await
                    .map_err(|err| AppError::Internal(err.into()))?;
                return Ok(Json(ClaimView::from_row(existing, true)));
            }
            tx.commit()
                .await
                .map_err(|err| AppError::Internal(err.into()))?;
            return Err(AppError::Conflict("not_complete".into()));
        }
        None => return Err(AppError::NotFound("quest")),
    };

    if view.state == QuestState::Claimed {
        let existing = PgQuestRepo::get_claim(&mut tx, auth.user_id, def.id, &view.period_id)
            .await?
            .ok_or_else(|| AppError::Internal(anyhow::anyhow!("claimed quest missing row")))?;
        tx.commit()
            .await
            .map_err(|err| AppError::Internal(err.into()))?;
        return Ok(Json(ClaimView::from_row(existing, true)));
    }

    if is_claimable(&snapshot, def.id).is_none() {
        tx.commit()
            .await
            .map_err(|err| AppError::Internal(err.into()))?;
        return Err(AppError::Conflict("not_complete".into()));
    }

    let inserted = PgQuestRepo::insert_claim(
        &mut tx,
        &NewQuestClaim {
            user_id: auth.user_id,
            quest_id: def.id.to_owned(),
            period_kind: def.category.as_str().to_owned(),
            period_id: view.period_id.clone(),
            season_id: season.as_ref().map(|s| s.id.as_i32()),
            reward_points: def.reward_points,
            catalog_version: catalog::VERSION,
        },
    )
    .await?;

    let (row, already) = if let Some(row) = inserted {
        (row, false)
    } else {
        let existing = PgQuestRepo::get_claim(&mut tx, auth.user_id, def.id, &view.period_id)
            .await?
            .ok_or_else(|| AppError::Internal(anyhow::anyhow!("claim conflict without row")))?;
        (existing, true)
    };

    tx.commit()
        .await
        .map_err(|err| AppError::Internal(err.into()))?;

    let mut redis = state.redis.clone();
    cache::invalidate(&mut redis, auth.user_id).await;
    realtime::publish_quest_updated(&state, auth.user_id);
    realtime::publish_leaderboard_updated(&state, season.as_ref().map(|s| s.id.as_i32()), "");

    Ok(Json(ClaimView::from_row(row, already)))
}
