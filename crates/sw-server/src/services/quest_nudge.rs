//! Daily quest reminder: claim send slots, then fan out OS push + WS.

use chrono::Utc;
use serde::Serialize;
use sw_domain::UserId;

use tracing::warn;

use crate::data::quest_nudges::{QuestNudgeRepo, unique_user_ids};
use crate::error::AppResult;
use crate::quests::period;
use crate::services::push::{
    QUEST_NUDGE_BODY, QUEST_NUDGE_PATH, QUEST_NUDGE_TITLE, quest_nudge_tag,
};
use crate::services::realtime;
use crate::state::AppState;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyNudgeResult {
    pub period_id: String,
    pub targeted: usize,
}

/// Insert today's nudge rows and spawn fanout. Safe to retry: unique (user, day).
pub async fn start(state: AppState) -> AppResult<DailyNudgeResult> {
    let clock = period::daily(Utc::now());
    let period_id = clock.id.clone();
    let repo = QuestNudgeRepo::new(state.db.clone());
    let push_on = state.push.enabled();
    let subs = if push_on {
        repo.claim_and_list(&period_id, clock.starts_at).await?
    } else {
        warn!("web push disabled; daily quest nudge will not claim send slots");
        repo.list_eligible(&period_id, clock.starts_at).await?
    };
    let user_ids = unique_user_ids(&subs);
    let targeted = user_ids.len();

    if !subs.is_empty() {
        tokio::spawn(async move {
            dispatch(state, subs, user_ids, period_id).await;
        });
    }

    Ok(DailyNudgeResult {
        period_id: clock.id,
        targeted,
    })
}

async fn dispatch(
    state: AppState,
    subs: Vec<crate::data::push::PushSubscription>,
    user_ids: Vec<uuid::Uuid>,
    period_id: String,
) {
    let tag = quest_nudge_tag(&period_id);
    if let Some(payload) = state.push.quest_nudge_payload(&period_id) {
        state
            .push
            .send_to_subscriptions(state.db.clone(), subs, payload)
            .await;
    }

    for id in user_ids {
        realtime::publish_user_notice(
            &state.sessions,
            &state.subscriptions,
            UserId::from(id),
            QUEST_NUDGE_TITLE,
            QUEST_NUDGE_BODY,
            QUEST_NUDGE_PATH,
            &tag,
        );
    }
}
