use thiserror::Error;

#[derive(Debug, Error)]
pub enum PluginError {
    #[error("unknown game id: {0}")]
    UnknownGame(String),

    #[error("game already registered: {0}")]
    AlreadyRegistered(String),

    #[error("engine error: {0}")]
    Engine(String),

    #[error("host error: {0}")]
    Host(String),

    #[error("invalid lobby configuration: {0}")]
    InvalidConfig(String),
}

pub type PluginResult<T> = Result<T, PluginError>;
