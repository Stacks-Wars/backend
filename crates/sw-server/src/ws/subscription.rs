use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use parking_lot::RwLock;
use sw_domain::UserId;
use tracing::debug;

use super::protocol::{MAX_TOPICS_PER_CONNECTION, ServerMessage};
use super::session::{ConnectionId, SessionManager};

#[derive(Debug, Default)]
pub struct SubscriptionManager {
    /// topic → subscribed connections
    by_topic: RwLock<HashMap<String, HashSet<ConnectionId>>>,
    /// connection → topics (for caps + cleanup)
    by_connection: RwLock<HashMap<ConnectionId, HashSet<String>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscribeError {
    TopicLimitReached,
}

impl SubscriptionManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn arc() -> Arc<Self> {
        Arc::new(Self::new())
    }

    pub fn subscribe(
        &self,
        connection_id: ConnectionId,
        topic: impl Into<String>,
    ) -> Result<(), SubscribeError> {
        let topic = topic.into();

        {
            let by_connection = self.by_connection.read();
            if let Some(topics) = by_connection.get(&connection_id) {
                if !topics.contains(&topic) && topics.len() >= MAX_TOPICS_PER_CONNECTION {
                    return Err(SubscribeError::TopicLimitReached);
                }
            }
        }

        self.by_topic
            .write()
            .entry(topic.clone())
            .or_default()
            .insert(connection_id);

        self.by_connection
            .write()
            .entry(connection_id)
            .or_default()
            .insert(topic.clone());

        debug!(%connection_id, %topic, "ws subscribed");
        Ok(())
    }

    pub fn unsubscribe(&self, connection_id: ConnectionId, topic: &str) -> bool {
        let removed = {
            let mut by_connection = self.by_connection.write();
            let removed = by_connection
                .get_mut(&connection_id)
                .is_some_and(|topics| topics.remove(topic));
            if let Some(topics) = by_connection.get(&connection_id) {
                if topics.is_empty() {
                    by_connection.remove(&connection_id);
                }
            }
            removed
        };

        {
            let mut by_topic = self.by_topic.write();
            if let Some(members) = by_topic.get_mut(topic) {
                members.remove(&connection_id);
                if members.is_empty() {
                    by_topic.remove(topic);
                }
            }
        }

        if removed {
            debug!(%connection_id, %topic, "ws unsubscribed");
        }
        removed
    }

    pub fn unsubscribe_all(&self, connection_id: ConnectionId) {
        let topics = self
            .by_connection
            .write()
            .remove(&connection_id)
            .unwrap_or_default();

        if topics.is_empty() {
            return;
        }

        let mut by_topic = self.by_topic.write();
        for topic in topics {
            if let Some(members) = by_topic.get_mut(&topic) {
                members.remove(&connection_id);
                if members.is_empty() {
                    by_topic.remove(&topic);
                }
            }
        }

        debug!(%connection_id, "ws unsubscribed all topics");
    }

    pub fn publish(&self, sessions: &SessionManager, topic: &str, message: ServerMessage) {
        let recipients: Vec<ConnectionId> = self
            .by_topic
            .read()
            .get(topic)
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default();

        for connection_id in recipients {
            let _ = sessions.send(connection_id, message.clone());
        }
    }

    /// Publish to every subscriber of `topic` except connections bound to `except`.
    pub fn publish_except(
        &self,
        sessions: &SessionManager,
        topic: &str,
        except: UserId,
        message: ServerMessage,
    ) {
        let recipients: Vec<ConnectionId> = self
            .by_topic
            .read()
            .get(topic)
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default();

        for connection_id in recipients {
            if sessions.user_id(connection_id) == Some(except) {
                continue;
            }
            let _ = sessions.send(connection_id, message.clone());
        }
    }

    pub fn members(&self, topic: &str) -> Vec<ConnectionId> {
        self.by_topic
            .read()
            .get(topic)
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Topics a connection is subscribed to.
    pub fn topics_for(&self, connection_id: ConnectionId) -> Vec<String> {
        self.by_connection
            .read()
            .get(&connection_id)
            .map(|set| set.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub fn topic_count(&self) -> usize {
        self.by_topic.read().len()
    }

    pub fn connection_topic_count(&self, connection_id: ConnectionId) -> usize {
        self.by_connection
            .read()
            .get(&connection_id)
            .map(HashSet::len)
            .unwrap_or(0)
    }
}
