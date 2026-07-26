use async_trait::async_trait;
use sw_domain::{User, UserId};

use crate::error::{AppError, AppResult};

#[async_trait]
pub trait UserRepo: Send + Sync {
    async fn get(&self, id: UserId) -> AppResult<Option<User>>;
    async fn upsert(&self, user: &User) -> AppResult<()>;
}

/// Placeholder repo that always reports not implemented.
pub struct StubUserRepo;

#[async_trait]
impl UserRepo for StubUserRepo {
    async fn get(&self, _id: UserId) -> AppResult<Option<User>> {
        Err(AppError::NotImplemented("UserRepo::get"))
    }

    async fn upsert(&self, _user: &User) -> AppResult<()> {
        Err(AppError::NotImplemented("UserRepo::upsert"))
    }
}
