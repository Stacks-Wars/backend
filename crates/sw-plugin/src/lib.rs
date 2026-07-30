//! Portable game plugin contract.
//!
//! Game crates depend on this crate (and `sw-domain`) — never on `sw-server`.

mod dto;
mod engine;
mod error;
mod factory;
mod game_error;
mod host;
pub mod kit;
mod message;
mod nop_host;
mod registry;
mod wire;

pub use dto::{JoinRequestState, PlayerStateWire, PlayerStatus};
pub use engine::{EngineContext, GameAction, GameEngine, GameEvent};
pub use error::{PluginError, PluginResult};
pub use factory::GameFactory;
pub use game_error::GameError;
pub use host::{GameHost, GameHostRef};
pub use kit::{
    calculate_wars_point, GameBootstrap, GamePlayerState, GameResults, GameStatus, GameSummary,
    PlayerRanking, PlayerResult, TurnRotation, WarsPointContext,
};
pub use message::MatchResult;
pub use nop_host::NopHost;
pub use registry::GameRegistry;
pub use wire::{GameRoomBroadcast, UserRoomMessage};
