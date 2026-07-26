use async_trait::async_trait;
use sw_domain::{Season, SeasonId, WarsPoints};

use crate::error::{AppError, AppResult};

#[async_trait]
pub trait SeasonRepo: Send + Sync {
    async fn current(&self) -> AppResult<Option<Season>>;
    async fn get(&self, id: SeasonId) -> AppResult<Option<Season>>;
    async fn points(&self, season_id: SeasonId) -> AppResult<Vec<WarsPoints>>;
}

pub struct StubSeasonRepo;

#[async_trait]
impl SeasonRepo for StubSeasonRepo {
    async fn current(&self) -> AppResult<Option<Season>> {
        Err(AppError::NotImplemented("SeasonRepo::current"))
    }

    async fn get(&self, _id: SeasonId) -> AppResult<Option<Season>> {
        Err(AppError::NotImplemented("SeasonRepo::get"))
    }

    async fn points(&self, _season_id: SeasonId) -> AppResult<Vec<WarsPoints>> {
        Err(AppError::NotImplemented("SeasonRepo::points"))
    }
}
