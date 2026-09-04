//! Best-effort Web Push. Missing VAPID keys → no-op.

use std::sync::Arc;

use futures::stream::{self, StreamExt};
use serde_json::{Value, json};
use tracing::{debug, warn};
use uuid::Uuid;
use web_push::{
    ContentEncoding, IsahcWebPushClient, PartialVapidSignatureBuilder, SubscriptionInfo,
    URL_SAFE_NO_PAD, Urgency, VapidSignatureBuilder, WebPushClient, WebPushError,
    WebPushMessageBuilder,
};

use crate::config::Config;
use crate::data::push::{PushSubscription, PushSubscriptionRepo};
use sw_domain::{ChainId, UserId};

const PUSH_CONCURRENCY: usize = 16;

#[derive(Clone)]
pub struct PushService {
    inner: Option<Arc<PushInner>>,
}

struct PushInner {
    vapid: PartialVapidSignatureBuilder,
    subject: String,
    frontend_url: String,
    client: IsahcWebPushClient,
}

pub fn lobby_notification_tag(path: &str) -> String {
    format!("lobby:{path}")
}

pub fn quest_nudge_tag(period_id: &str) -> String {
    format!("quest:daily:{period_id}")
}

pub const QUEST_NUDGE_TITLE: &str = "You have a new quest today";
pub const QUEST_NUDGE_BODY: &str = "Don't lose your streak.";
pub const QUEST_NUDGE_PATH: &str = "/quests";
pub const QUEST_NUDGE_CTA: &str = "See quest";

impl PushService {
    pub fn from_config(config: &Config) -> Self {
        match (
            config.vapid_public_key.as_deref(),
            config.vapid_private_key.as_deref(),
        ) {
            (Some(_public_key), Some(private_key)) if !private_key.is_empty() => {
                match (
                    VapidSignatureBuilder::from_base64_no_sub(private_key, URL_SAFE_NO_PAD),
                    IsahcWebPushClient::new(),
                ) {
                    (Ok(vapid), Ok(client)) => Self {
                        inner: Some(Arc::new(PushInner {
                            vapid,
                            subject: config.vapid_subject.clone(),
                            frontend_url: config.frontend_url.clone(),
                            client,
                        })),
                    },
                    (Err(err), _) => {
                        warn!(error = %err, "invalid VAPID private key");
                        Self { inner: None }
                    }
                    (_, Err(err)) => {
                        warn!(error = %err, "web-push client unavailable");
                        Self { inner: None }
                    }
                }
            }
            _ => Self { inner: None },
        }
    }

    pub fn enabled(&self) -> bool {
        self.inner.is_some()
    }

    pub async fn send_to_user(
        &self,
        db: sqlx::PgPool,
        user_id: UserId,
        title: &str,
        body: &str,
        path: &str,
    ) {
        let Some(inner) = self.inner.clone() else {
            return;
        };
        let Ok(subs) = PushSubscriptionRepo::new(db.clone())
            .list_for_user(user_id)
            .await
        else {
            return;
        };
        inner
            .dispatch(
                &db,
                &subs,
                &json!({
                    "title": title,
                    "body": body,
                    "url": format!("{}{}", inner.frontend_url, path),
                    "silent": false,
                }),
            )
            .await;
    }

    /// Fan out one payload to a preloaded subscription list (cron / batch).
    pub async fn send_to_subscriptions(
        &self,
        db: sqlx::PgPool,
        subs: Vec<PushSubscription>,
        payload: Value,
    ) {
        let Some(inner) = self.inner.clone() else {
            return;
        };
        inner.dispatch_concurrent(&db, &subs, &payload).await;
    }

    pub fn quest_nudge_payload(&self, period_id: &str) -> Option<Value> {
        let inner = self.inner.as_ref()?;
        Some(json!({
            "title": QUEST_NUDGE_TITLE,
            "body": QUEST_NUDGE_BODY,
            "url": format!("{}{}", inner.frontend_url, QUEST_NUDGE_PATH),
            "tag": quest_nudge_tag(period_id),
            "silent": false,
            "actions": [{ "action": "open", "title": QUEST_NUDGE_CTA }],
        }))
    }

    pub async fn send_lobby_created(
        &self,
        db: sqlx::PgPool,
        creator_id: UserId,
        lobby_name: &str,
        lobby_path: &str,
        game_id: &str,
        chain: ChainId,
        entry_amount_micro: i64,
    ) {
        let Some(inner) = self.inner.clone() else {
            return;
        };
        let paid_chain = (entry_amount_micro > 0).then_some(chain);
        let Ok(subs) = PushSubscriptionRepo::new(db.clone())
            .list_lobby_alert_targets(creator_id, paid_chain)
            .await
        else {
            return;
        };
        let tag = lobby_notification_tag(lobby_path);
        inner
            .dispatch(
                &db,
                &subs,
                &json!({
                    "title": "New lobby",
                    "body": format!("{lobby_name} · {game_id}"),
                    "url": format!("{}/room/{}", inner.frontend_url, lobby_path),
                    "tag": tag,
                    "silent": false,
                }),
            )
            .await;
    }

    /// Retract the lobby.created OS banner once the lobby is gone or finished.
    pub async fn close_lobby(
        &self,
        db: sqlx::PgPool,
        creator_id: UserId,
        lobby_path: &str,
        chain: ChainId,
        entry_amount_micro: i64,
    ) {
        let Some(inner) = self.inner.clone() else {
            return;
        };
        let paid_chain = (entry_amount_micro > 0).then_some(chain);
        let Ok(subs) = PushSubscriptionRepo::new(db.clone())
            .list_lobby_alert_targets(creator_id, paid_chain)
            .await
        else {
            return;
        };
        inner
            .dispatch(
                &db,
                &subs,
                &json!({
                    "action": "close",
                    "tag": lobby_notification_tag(lobby_path),
                }),
            )
            .await;
    }
}

impl PushInner {
    async fn dispatch(&self, db: &sqlx::PgPool, subs: &[PushSubscription], payload: &Value) {
        for sub in subs {
            self.handle_send_result(db, sub, self.send_one(sub, payload).await)
                .await;
        }
    }

    async fn dispatch_concurrent(
        &self,
        db: &sqlx::PgPool,
        subs: &[PushSubscription],
        payload: &Value,
    ) {
        stream::iter(subs)
            .for_each_concurrent(PUSH_CONCURRENCY, |sub| async move {
                self.handle_send_result(db, sub, self.send_one(sub, payload).await)
                    .await;
            })
            .await;
    }

    async fn handle_send_result(
        &self,
        db: &sqlx::PgPool,
        sub: &PushSubscription,
        result: Result<(), WebPushError>,
    ) {
        match result {
            Ok(()) => {}
            Err(WebPushError::EndpointNotValid | WebPushError::EndpointNotFound) => {
                let _ = PushSubscriptionRepo::new(db.clone())
                    .delete_endpoint(UserId::from(sub.user_id), &sub.endpoint)
                    .await;
            }
            Err(err) => {
                debug!(endpoint = %sub.endpoint, error = %err, "web-push send failed");
            }
        }
    }

    async fn send_one(&self, sub: &PushSubscription, payload: &Value) -> Result<(), WebPushError> {
        let info = SubscriptionInfo::new(&sub.endpoint, &sub.p256dh, &sub.auth);
        let mut sig = self.vapid.clone().add_sub_info(&info);
        sig.add_claim("sub", self.subject.as_str());
        let mut builder = WebPushMessageBuilder::new(&info);
        let bytes = serde_json::to_vec(payload).map_err(|_| WebPushError::InvalidResponse)?;
        builder.set_payload(ContentEncoding::Aes128Gcm, &bytes);
        builder.set_urgency(Urgency::High);
        builder.set_vapid_signature(sig.build()?);
        self.client.send(builder.build()?).await
    }
}

/// Fire-and-forget so HTTP handlers never wait on push fanout.
pub fn spawn_lobby_created(
    push: PushService,
    db: sqlx::PgPool,
    creator_id: UserId,
    lobby_name: String,
    lobby_path: String,
    game_id: String,
    chain: ChainId,
    entry_amount_micro: i64,
) {
    tokio::spawn(async move {
        push.send_lobby_created(
            db,
            creator_id,
            &lobby_name,
            &lobby_path,
            &game_id,
            chain,
            entry_amount_micro,
        )
        .await;
    });
}

pub fn spawn_lobby_close(
    push: PushService,
    db: sqlx::PgPool,
    creator_id: UserId,
    lobby_path: String,
    chain: ChainId,
    entry_amount_micro: i64,
) {
    tokio::spawn(async move {
        push.close_lobby(db, creator_id, &lobby_path, chain, entry_amount_micro)
            .await;
    });
}

pub fn spawn_user_notice(
    push: PushService,
    db: sqlx::PgPool,
    user_id: UserId,
    title: String,
    body: String,
    path: String,
) {
    tokio::spawn(async move {
        push.send_to_user(db, user_id, &title, &body, &path).await;
    });
}

pub fn spawn_users_notice(
    push: PushService,
    db: sqlx::PgPool,
    user_ids: Vec<Uuid>,
    title: String,
    body: String,
    path: String,
) {
    tokio::spawn(async move {
        for id in user_ids {
            push.send_to_user(db.clone(), UserId::from(id), &title, &body, &path)
                .await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quest_tag_includes_period() {
        assert_eq!(quest_nudge_tag("2026-09-01"), "quest:daily:2026-09-01");
    }
}
