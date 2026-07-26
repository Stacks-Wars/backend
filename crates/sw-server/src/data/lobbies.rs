use async_trait::async_trait;
use sw_domain::{Lobby, LobbyId};

use crate::error::{AppError, AppResult};

#[async_trait]
pub trait LobbyRepo: Send + Sync {
    async fn get(&self, id: LobbyId) -> AppResult<Option<Lobby>>;
    async fn save(&self, lobby: &Lobby) -> AppResult<()>;
}

pub struct StubLobbyRepo;

#[async_trait]
impl LobbyRepo for StubLobbyRepo {
    async fn get(&self, _id: LobbyId) -> AppResult<Option<Lobby>> {
        Err(AppError::NotImplemented("LobbyRepo::get"))
    }

    async fn save(&self, _lobby: &Lobby) -> AppResult<()> {
        Err(AppError::NotImplemented("LobbyRepo::save"))
    }
}
