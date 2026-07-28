use thiserror::Error;

#[derive(Debug, Error, Clone)]
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

    #[error("serialization error: {0}")]
    Serialization(String),

    #[error("deserialization error: {0}")]
    Deserialization(String),

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("internal error")]
    Internal,
}

pub type PluginResult<T> = Result<T, PluginError>;
