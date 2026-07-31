//! Minimal Telegram Bot API client over reqwest.

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use tracing::warn;

#[derive(Debug, Clone)]
pub struct TelegramClient {
    http: reqwest::Client,
    base: String,
}

#[derive(Debug, Deserialize)]
struct ApiResponse<T> {
    ok: bool,
    result: Option<T>,
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Message {
    pub message_id: i64,
}

#[derive(Debug, Deserialize)]
pub struct Update {
    pub update_id: i64,
    pub message: Option<IncomingMessage>,
}

#[derive(Debug, Deserialize)]
pub struct IncomingMessage {
    #[allow(dead_code)]
    pub message_id: i64,
    pub chat: Chat,
    pub text: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Chat {
    pub id: i64,
}

impl TelegramClient {
    pub fn new(token: String) -> Self {
        Self {
            http: reqwest::Client::new(),
            base: format!("https://api.telegram.org/bot{token}"),
        }
    }

    async fn call<T: for<'de> Deserialize<'de>>(
        &self,
        method: &str,
        body: Value,
    ) -> Result<T> {
        let url = format!("{}/{method}", self.base);
        let resp = self
            .http
            .post(url)
            .json(&body)
            .send()
            .await
            .context("telegram http")?;
        let status = resp.status();
        let parsed: ApiResponse<T> = resp.json().await.context("telegram decode")?;
        if !parsed.ok || !status.is_success() {
            return Err(anyhow!(
                "telegram {method} failed: {}",
                parsed.description.unwrap_or_else(|| status.to_string())
            ));
        }
        parsed
            .result
            .ok_or_else(|| anyhow!("telegram {method}: empty result"))
    }

    pub async fn send_message(
        &self,
        chat_id: i64,
        text: &str,
        reply_markup: Option<Value>,
        reply_to_message_id: Option<i64>,
        // When set, Telegram previews this URL (buttons cannot drive previews)
        // and `show_above_text` places it above the message body.
        preview_url: Option<&str>,
    ) -> Result<Message> {
        let mut body = json!({
            "chat_id": chat_id,
            "text": text,
            "parse_mode": "HTML",
        });
        if let Some(url) = preview_url.filter(|u| !u.is_empty()) {
            body["link_preview_options"] = json!({
                "is_disabled": false,
                "url": url,
                "prefer_large_media": true,
                "show_above_text": true,
            });
        } else {
            body["link_preview_options"] = json!({ "is_disabled": true });
        }
        if let Some(markup) = reply_markup {
            body["reply_markup"] = markup;
        }
        if let Some(reply_to) = reply_to_message_id {
            body["reply_parameters"] = json!({ "message_id": reply_to });
        }
        self.call("sendMessage", body).await
    }

    pub async fn delete_message(&self, chat_id: i64, message_id: i64) -> Result<()> {
        let _: Value = self
            .call(
                "deleteMessage",
                json!({ "chat_id": chat_id, "message_id": message_id }),
            )
            .await?;
        Ok(())
    }

    pub async fn edit_message_text(
        &self,
        chat_id: i64,
        message_id: i64,
        text: &str,
    ) -> Result<()> {
        let _: Value = self
            .call(
                "editMessageText",
                json!({
                    "chat_id": chat_id,
                    "message_id": message_id,
                    "text": text,
                    "parse_mode": "HTML",
                    "disable_web_page_preview": false,
                }),
            )
            .await?;
        Ok(())
    }

    pub async fn get_updates(&self, offset: i64, timeout_secs: u64) -> Result<Vec<Update>> {
        self.call(
            "getUpdates",
            json!({
                "offset": offset,
                "timeout": timeout_secs,
                "allowed_updates": ["message"],
            }),
        )
        .await
    }
}

/// Long-poll for `/leaderboard` (and `/start` help).
pub async fn run_command_loop(
    notifier: std::sync::Arc<super::notify::TelegramNotifier>,
    state: crate::state::AppState,
) -> Result<()> {
    let client = notifier
        .client
        .as_ref()
        .ok_or_else(|| anyhow!("telegram disabled"))?
        .clone();

    let mut offset: i64 = 0;
    loop {
        let updates = match client.get_updates(offset, 25).await {
            Ok(u) => u,
            Err(err) => {
                warn!(error = %err, "telegram getUpdates failed");
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                continue;
            }
        };
        for update in updates {
            offset = offset.max(update.update_id + 1);
            let Some(msg) = update.message else { continue };
            let Some(text) = msg.text.as_deref() else { continue };
            let cmd = text.split_whitespace().next().unwrap_or("");
            let cmd = cmd.split('@').next().unwrap_or(cmd).to_ascii_lowercase();
            match cmd.as_str() {
                "/leaderboard" => {
                    let reply_chat = msg.chat.id;
                    let state = state.clone();
                    let notifier = notifier.clone();
                    tokio::spawn(async move {
                        if let Err(err) =
                            notifier.reply_leaderboard(&state, reply_chat).await
                        {
                            warn!(error = %err, "telegram /leaderboard failed");
                        }
                    });
                }
                "/start" | "/help" => {
                    let _ = client
                        .send_message(
                            msg.chat.id,
                            "Stacks Wars bot\n\nCommands:\n/leaderboard — current season top 10",
                            None,
                            None,
                            None,
                        )
                        .await;
                }
                _ => {}
            }
        }
    }
}
