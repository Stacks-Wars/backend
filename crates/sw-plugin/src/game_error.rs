use crate::PluginError;
use std::fmt;

#[derive(Debug, Clone)]
pub enum GameError {
    NotYourTurn,
    NotInGame,
    GameFinished,
    GameNotStarted,
    InvalidAction(String),
    AlreadyEliminated,
    InsufficientPlayers { required: usize, actual: usize },
    Internal(String),
}

impl fmt::Display for GameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GameError::NotYourTurn => write!(f, "Not your turn"),
            GameError::NotInGame => write!(f, "You are not in this game"),
            GameError::GameFinished => write!(f, "Game has already finished"),
            GameError::GameNotStarted => write!(f, "Game has not started yet"),
            GameError::InvalidAction(msg) => write!(f, "Invalid action: {msg}"),
            GameError::AlreadyEliminated => write!(f, "You have been eliminated"),
            GameError::InsufficientPlayers { required, actual } => {
                write!(f, "Need at least {required} players, got {actual}")
            }
            GameError::Internal(msg) => write!(f, "Internal game error: {msg}"),
        }
    }
}

impl std::error::Error for GameError {}

impl From<GameError> for PluginError {
    fn from(err: GameError) -> Self {
        match err {
            GameError::Internal(_) => PluginError::Internal,
            other => PluginError::BadRequest(other.to_string()),
        }
    }
}
