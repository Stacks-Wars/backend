//! Multiplexed `/app` WebSocket: sessions, subscriptions, and the connection handler.

mod handler;
mod protocol;
mod session;
mod subscription;

pub use handler::router;
pub use protocol::{
    ALL_FEED_TOPIC, APP_TOPIC, ClientMessage, ServerMessage, chain_feed_topic,
    parse_chain_feed_topic,
};
pub use session::{ConnectionId, SessionManager};
pub use subscription::{SubscribeError, SubscriptionManager};
