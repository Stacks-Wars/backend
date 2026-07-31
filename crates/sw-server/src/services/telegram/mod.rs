//! Telegram companion — lobby announcements + `/leaderboard`.
//!
//! Disabled when `TELEGRAM_BOT_TOKEN` / `TELEGRAM_CHAT_ID` are unset.
//! Never includes emails, UUIDs, or wallet addresses in outbound text.

mod client;
mod format;
mod notify;

pub use notify::TelegramNotifier;

use std::sync::Arc;

use tracing::{info, warn};

use crate::config::Config;
use crate::state::AppState;

use self::client::TelegramClient;

/// Spawn long-polling for bot commands when Telegram is configured.
pub fn spawn_bot(state: AppState) {
    let Some(notifier) = state.telegram.clone().into_enabled() else {
        info!("telegram bot disabled (no TELEGRAM_BOT_TOKEN)");
        return;
    };
    tokio::spawn(async move {
        if let Err(err) = client::run_command_loop(notifier, state).await {
            warn!(error = %err, "telegram command loop exited");
        }
    });
}

impl TelegramNotifier {
    pub fn from_config(config: &Config) -> Arc<Self> {
        match (&config.telegram_bot_token, config.telegram_chat_id) {
            (Some(token), Some(chat_id)) => Arc::new(Self {
                client: Some(TelegramClient::new(token.clone())),
                chat_id,
                frontend_url: config.frontend_url.clone(),
            }),
            _ => Arc::new(Self {
                client: None,
                chat_id: 0,
                frontend_url: config.frontend_url.clone(),
            }),
        }
    }

    fn into_enabled(self: Arc<Self>) -> Option<Arc<Self>> {
        if self.client.is_some() {
            Some(self)
        } else {
            None
        }
    }

    pub fn enabled(&self) -> bool {
        self.client.is_some()
    }
}
