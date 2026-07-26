//! Portable game plugin contract.
//!
//! Game crates depend on this crate (and optionally `sw-domain`) — never on
//! `sw-server`. The server hosts engines behind [`GameHost`] and discovers
//! them via [`GameFactory`] entries in a [`GameRegistry`].

mod engine;
mod error;
mod factory;
mod host;
mod message;
mod registry;

pub use engine::*;
pub use error::*;
pub use factory::*;
pub use host::*;
pub use message::*;
pub use registry::*;
