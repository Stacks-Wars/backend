//! Per-lobby game engine actors.
//!
//! `POST /lobbies/{id}/start` builds a [`GameEngine`] and hands it to
//! [`EngineRegistry::spawn`], which owns it for the rest of the match. The
//! engine is only reachable through its mailbox, so WebSocket `game.action`
//! frames and state snapshots are serialized against the engine's own loop.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::RwLock;
use serde_json::Value;
use sw_domain::{GameId, LobbyId, UserId};
use sw_plugin::{GameEngine, GameHostRef};
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, error, info, warn};

/// Pending commands per lobby before senders start failing fast.
const MAILBOX_CAPACITY: usize = 256;
/// How often the actor re-checks `is_finished` when no commands arrive.
const FINISH_POLL: Duration = Duration::from_secs(3);
/// Upper bound on a `get_game_state` round trip.
const SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(3);

pub enum EngineCommand {
    Action {
        user_id: UserId,
        action: Value,
    },
    Quit {
        user_id: UserId,
    },
    Snapshot {
        user_id: Option<UserId>,
        reply: oneshot::Sender<Value>,
    },
    Shutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchError {
    /// No engine is running for this lobby.
    NotRunning,
    /// Engine mailbox is full or the actor has stopped.
    Unavailable,
}

impl DispatchError {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotRunning => "no running game for this lobby",
            Self::Unavailable => "game is not accepting actions right now",
        }
    }
}

#[derive(Clone)]
pub struct EngineHandle {
    game_id: GameId,
    tx: mpsc::Sender<EngineCommand>,
}

impl EngineHandle {
    pub fn game_id(&self) -> &GameId {
        &self.game_id
    }

    pub fn send_action(&self, user_id: UserId, action: Value) -> Result<(), DispatchError> {
        self.tx
            .try_send(EngineCommand::Action { user_id, action })
            .map_err(|_| DispatchError::Unavailable)
    }

    pub fn send_quit(&self, user_id: UserId) -> Result<(), DispatchError> {
        self.tx
            .try_send(EngineCommand::Quit { user_id })
            .map_err(|_| DispatchError::Unavailable)
    }

    /// Current engine view for `user_id`. `None` if the engine is gone or slow.
    pub async fn snapshot(&self, user_id: Option<UserId>) -> Option<Value> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(EngineCommand::Snapshot { user_id, reply })
            .await
            .ok()?;
        match tokio::time::timeout(SNAPSHOT_TIMEOUT, rx).await {
            Ok(Ok(value)) => Some(value),
            _ => None,
        }
    }

    pub async fn shutdown(&self) {
        let _ = self.tx.send(EngineCommand::Shutdown).await;
    }
}

#[derive(Default)]
pub struct EngineRegistry {
    engines: RwLock<HashMap<LobbyId, EngineHandle>>,
}

impl EngineRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn arc() -> Arc<Self> {
        Arc::new(Self::new())
    }

    pub fn get(&self, lobby_id: LobbyId) -> Option<EngineHandle> {
        self.engines.read().get(&lobby_id).cloned()
    }

    pub fn is_running(&self, lobby_id: LobbyId) -> bool {
        self.engines.read().contains_key(&lobby_id)
    }

    pub fn len(&self) -> usize {
        self.engines.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn remove(&self, lobby_id: LobbyId) {
        self.engines.write().remove(&lobby_id);
    }

    /// Take ownership of `engine` and drive it until the match ends.
    ///
    /// Returns `false` when a match is already running for `lobby_id`.
    pub fn spawn(
        self: &Arc<Self>,
        lobby_id: LobbyId,
        mut engine: Box<dyn GameEngine>,
        host: GameHostRef,
    ) -> bool {
        if self.is_running(lobby_id) {
            warn!(%lobby_id, "engine already running; refusing to spawn a second one");
            return false;
        }

        let (tx, mut rx) = mpsc::channel(MAILBOX_CAPACITY);
        let game_id = engine.game_id().clone();
        self.engines.write().insert(
            lobby_id,
            EngineHandle {
                game_id: game_id.clone(),
                tx,
            },
        );

        let registry = Arc::clone(self);
        tokio::spawn(async move {
            info!(%lobby_id, %game_id, "engine started");

            if let Err(err) = engine.start(host.clone()).await {
                error!(%lobby_id, %game_id, error = %err, "engine start failed");
            }

            let mut ticker = tokio::time::interval(FINISH_POLL);
            ticker.tick().await;

            loop {
                if engine.is_finished() {
                    break;
                }

                tokio::select! {
                    received = rx.recv() => {
                        let Some(command) = received else { break };
                        match command {
                            EngineCommand::Action { user_id, action } => {
                                if let Err(err) =
                                    engine.handle_action(host.clone(), user_id, action).await
                                {
                                    debug!(
                                        %lobby_id, %user_id, error = %err,
                                        "engine rejected action"
                                    );
                                }
                            }
                            EngineCommand::Quit { user_id } => {
                                if let Err(err) =
                                    engine.handle_player_quit(host.clone(), user_id).await
                                {
                                    warn!(%lobby_id, %user_id, error = %err, "quit failed");
                                }
                            }
                            EngineCommand::Snapshot { user_id, reply } => {
                                let value = engine
                                    .get_game_state(user_id)
                                    .await
                                    .unwrap_or(Value::Null);
                                let _ = reply.send(value);
                            }
                            EngineCommand::Shutdown => break,
                        }
                    }
                    _ = ticker.tick() => {}
                }
            }

            if let Err(err) = engine.shutdown(host).await {
                warn!(%lobby_id, %game_id, error = %err, "engine shutdown failed");
            }
            registry.remove(lobby_id);
            info!(%lobby_id, %game_id, "engine stopped");
        });

        true
    }
}
