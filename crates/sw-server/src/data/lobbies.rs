use chrono::{DateTime, Utc};
use sqlx::PgPool;
use sw_domain::{GameId, Lobby, LobbyId, UserId, WalletAddress};
use uuid::Uuid;

use crate::data::lobby_status::DbLobbyStatus;
use crate::error::{AppError, AppResult};

#[derive(Debug, sqlx::FromRow)]
struct LobbyRow {
    id: Uuid,
    path: String,
    name: String,
    description: Option<String>,
    game_id: String,
    creator_id: Uuid,
    entry_amount: Option<f64>,
    current_amount: Option<f64>,
    contract_address: Option<String>,
    is_private: bool,
    is_sponsored: bool,
    status: DbLobbyStatus,
    participants: Vec<Uuid>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl LobbyRow {
    fn into_lobby(self) -> AppResult<Lobby> {
        Ok(Lobby {
            id: LobbyId::from(self.id),
            path: self.path,
            name: self.name,
            description: self.description,
            game_id: GameId::new(self.game_id).map_err(|e| AppError::BadRequest(e.to_string()))?,
            creator_id: UserId::from(self.creator_id),
            entry_amount: self.entry_amount,
            current_amount: self.current_amount,
            contract_address: self.contract_address.map(WalletAddress::from),
            is_private: self.is_private,
            is_sponsored: self.is_sponsored,
            status: self.status.into(),
            created_at: self.created_at,
            updated_at: self.updated_at,
            participants: self.participants.into_iter().map(UserId::from).collect(),
        })
    }
}

pub struct PgLobbyRepo {
    pool: PgPool,
}

impl PgLobbyRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn insert(&self, lobby: &Lobby) -> AppResult<()> {
        let participant_uuids: Vec<Uuid> = lobby
            .participants
            .iter()
            .map(|id| id.as_uuid())
            .collect();
        let status = DbLobbyStatus::from(lobby.status);
        let contract = lobby.contract_address.as_ref().map(|a| a.as_str());

        sqlx::query(
            r#"
            INSERT INTO lobbies (
                id, path, name, description, game_id, creator_id,
                entry_amount, current_amount, contract_address,
                is_private, is_sponsored, status, participants,
                created_at, updated_at
            ) VALUES (
                $1, $2, $3, $4, $5, $6,
                $7, $8, $9,
                $10, $11, $12, $13,
                $14, $15
            )
            "#,
        )
        .bind(lobby.id.as_uuid())
        .bind(&lobby.path)
        .bind(&lobby.name)
        .bind(&lobby.description)
        .bind(lobby.game_id.as_str())
        .bind(lobby.creator_id.as_uuid())
        .bind(lobby.entry_amount)
        .bind(lobby.current_amount)
        .bind(contract)
        .bind(lobby.is_private)
        .bind(lobby.is_sponsored)
        .bind(status)
        .bind(&participant_uuids)
        .bind(lobby.created_at)
        .bind(lobby.updated_at)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Internal(e.into()))?;

        Ok(())
    }

    pub async fn get_by_path(&self, path: &str) -> AppResult<Option<Lobby>> {
        let row = sqlx::query_as::<_, LobbyRow>(
            r#"
            SELECT id, path, name, description, game_id, creator_id,
                   entry_amount, current_amount, contract_address,
                   is_private, is_sponsored, status, participants,
                   created_at, updated_at
            FROM lobbies
            WHERE path = $1
            "#,
        )
        .bind(path)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::Internal(e.into()))?;

        row.map(LobbyRow::into_lobby).transpose()
    }

    pub async fn get_by_id(&self, id: LobbyId) -> AppResult<Option<Lobby>> {
        let row = sqlx::query_as::<_, LobbyRow>(
            r#"
            SELECT id, path, name, description, game_id, creator_id,
                   entry_amount, current_amount, contract_address,
                   is_private, is_sponsored, status, participants,
                   created_at, updated_at
            FROM lobbies
            WHERE id = $1
            "#,
        )
        .bind(id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::Internal(e.into()))?;

        row.map(LobbyRow::into_lobby).transpose()
    }

    pub async fn path_exists(&self, path: &str) -> AppResult<bool> {
        let exists: bool = sqlx::query_scalar(
            r#"SELECT EXISTS(SELECT 1 FROM lobbies WHERE path = $1)"#,
        )
        .bind(path)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::Internal(e.into()))?;
        Ok(exists)
    }
}

/// First 8 hex chars of a UUID without dashes.
pub fn lobby_path_from_uuid(id: Uuid) -> String {
    id.simple().to_string().chars().take(8).collect()
}

pub async fn generate_unique_lobby_path(repo: &PgLobbyRepo) -> AppResult<String> {
    for _ in 0..16 {
        let path = lobby_path_from_uuid(Uuid::now_v7());
        if !repo.path_exists(&path).await? {
            return Ok(path);
        }
    }
    Err(AppError::Internal(anyhow::anyhow!(
        "failed to allocate unique lobby path"
    )))
}
