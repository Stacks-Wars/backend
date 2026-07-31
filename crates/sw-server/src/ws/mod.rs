//! Multiplexed `/app` WebSocket: sessions, subscriptions, and the connection handler.

mod handler;
mod protocol;
mod session;
mod subscription;

pub use handler::router;
pub use protocol::{ClientMessage, ServerMessage, APP_TOPIC};
pub use session::{ConnectionId, SessionManager};
pub use subscription::{SubscribeError, SubscriptionManager};
