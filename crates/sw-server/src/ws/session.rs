use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;
use sw_domain::UserId;
use tokio::sync::mpsc;
use tracing::{debug, warn};
use uuid::Uuid;

use super::protocol::ServerMessage;

pub type ConnectionId = Uuid;

const OUTBOUND_BUFFER: usize = 64;

#[derive(Debug)]
pub struct Session {
    pub id: ConnectionId,
    pub user_id: Option<UserId>,
    tx: mpsc::Sender<ServerMessage>,
}

impl Session {
    pub fn sender(&self) -> mpsc::Sender<ServerMessage> {
        self.tx.clone()
    }
}

#[derive(Debug, Default)]
pub struct SessionManager {
    sessions: RwLock<HashMap<ConnectionId, Session>>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn arc() -> Arc<Self> {
        Arc::new(Self::new())
    }

    /// Register a new anonymous session and return `(id, outbound receiver)`.
    pub fn insert(&self, user_id: Option<UserId>) -> (ConnectionId, mpsc::Receiver<ServerMessage>) {
        let id = Uuid::now_v7();
        let (tx, rx) = mpsc::channel(OUTBOUND_BUFFER);
        let session = Session { id, user_id, tx };
        self.sessions.write().insert(id, session);
        debug!(%id, "ws session registered");
        (id, rx)
    }

    pub fn bind_user(&self, connection_id: ConnectionId, user_id: UserId) -> bool {
        let mut sessions = self.sessions.write();
        let Some(session) = sessions.get_mut(&connection_id) else {
            return false;
        };
        session.user_id = Some(user_id);
        true
    }

    pub fn user_id(&self, connection_id: ConnectionId) -> Option<UserId> {
        self.sessions
            .read()
            .get(&connection_id)
            .and_then(|s| s.user_id)
    }

    pub fn remove(&self, connection_id: ConnectionId) -> bool {
        let removed = self.sessions.write().remove(&connection_id).is_some();
        if removed {
            debug!(%connection_id, "ws session removed");
        }
        removed
    }

    pub fn len(&self) -> usize {
        self.sessions.read().len()
    }

    pub fn send(&self, connection_id: ConnectionId, message: ServerMessage) -> bool {
        let tx = {
            let sessions = self.sessions.read();
            sessions.get(&connection_id).map(|session| session.tx.clone())
        };

        let Some(tx) = tx else {
            return false;
        };

        match tx.try_send(message) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(_)) => {
                warn!(%connection_id, "ws outbound buffer full; dropping message");
                false
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                debug!(%connection_id, "ws outbound closed");
                false
            }
        }
    }
}
