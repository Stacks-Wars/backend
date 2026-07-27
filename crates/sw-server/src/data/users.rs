use chrono::{DateTime, Utc};
use sqlx::PgPool;
use sw_domain::{User, UserId};
use uuid::Uuid;

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone)]
pub struct UpsertUserInput {
    pub email: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub email_verified_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct CustodialWalletInput {
    pub stx_address: String,
    pub public_key: String,
    pub encrypted_mnemonic: String,
    pub kms_key_version: String,
    pub network: String,
}

/// Public custodial wallet fields for transaction flows (no key material).
#[derive(Debug, Clone)]
pub struct CustodialWalletPublic {
    pub user_id: Uuid,
    pub stx_address: String,
    pub public_key: String,
    pub network: String,
}

#[derive(Debug, sqlx::FromRow)]
struct UserRow {
    id: Uuid,
    username: Option<String>,
    display_name: Option<String>,
    email: String,
    email_verified_at: Option<DateTime<Utc>>,
    wallet_address: Option<String>,
    wallet_verified_at: Option<DateTime<Utc>>,
    avatar_url: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, sqlx::FromRow)]
struct CustodialWalletRow {
    user_id: Uuid,
    stx_address: String,
    public_key: String,
    network: String,
}

impl From<UserRow> for User {
    fn from(row: UserRow) -> Self {
        Self {
            id: UserId(row.id),
            username: row.username,
            display_name: row.display_name,
            email: row.email,
            email_verified_at: row.email_verified_at,
            wallet_address: row.wallet_address,
            wallet_verified_at: row.wallet_verified_at,
            avatar_url: row.avatar_url,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

impl From<CustodialWalletRow> for CustodialWalletPublic {
    fn from(row: CustodialWalletRow) -> Self {
        Self {
            user_id: row.user_id,
            stx_address: row.stx_address,
            public_key: row.public_key,
            network: row.network,
        }
    }
}

pub struct PgUserRepo {
    pool: PgPool,
}

impl PgUserRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn get_by_id(&self, id: UserId) -> AppResult<Option<User>> {
        let row = sqlx::query_as::<_, UserRow>(
            r#"
            SELECT id, username, display_name, email, email_verified_at,
                   wallet_address, wallet_verified_at, avatar_url,
                   created_at, updated_at
            FROM users
            WHERE id = $1
            "#,
        )
        .bind(id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| AppError::Internal(err.into()))?;

        Ok(row.map(User::from))
    }

    pub async fn upsert(&self, input: UpsertUserInput) -> AppResult<User> {
        let row = sqlx::query_as::<_, UserRow>(
            r#"
            INSERT INTO users (email, display_name, avatar_url, email_verified_at)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (email) DO UPDATE SET
                display_name = COALESCE(EXCLUDED.display_name, users.display_name),
                avatar_url = COALESCE(EXCLUDED.avatar_url, users.avatar_url),
                email_verified_at = COALESCE(EXCLUDED.email_verified_at, users.email_verified_at),
                updated_at = now()
            RETURNING id, username, display_name, email, email_verified_at,
                      wallet_address, wallet_verified_at, avatar_url,
                      created_at, updated_at
            "#,
        )
        .bind(&input.email)
        .bind(&input.display_name)
        .bind(&input.avatar_url)
        .bind(input.email_verified_at)
        .fetch_one(&self.pool)
        .await
        .map_err(|err| AppError::Internal(err.into()))?;

        Ok(User::from(row))
    }

    pub async fn get_custodial_wallet(
        &self,
        user_id: UserId,
    ) -> AppResult<Option<CustodialWalletPublic>> {
        let row = sqlx::query_as::<_, CustodialWalletRow>(
            r#"
            SELECT user_id, stx_address, public_key, network
            FROM custodial_wallets
            WHERE user_id = $1 AND status = 'active'
            LIMIT 1
            "#,
        )
        .bind(user_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| AppError::Internal(err.into()))?;

        Ok(row.map(CustodialWalletPublic::from))
    }

    pub async fn create_custodial_wallet(
        &self,
        user_id: UserId,
        wallet: CustodialWalletInput,
    ) -> AppResult<CustodialWalletPublic> {
        if self.get_by_id(user_id).await?.is_none() {
            return Err(AppError::NotFound("user"));
        }

        if let Some(existing) = self.get_custodial_wallet(user_id).await? {
            return Ok(existing);
        }

        sqlx::query(
            r#"
            INSERT INTO custodial_wallets (
                user_id, stx_address, public_key, encrypted_mnemonic,
                kms_key_version, network
            )
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (user_id) DO NOTHING
            "#,
        )
        .bind(user_id.as_uuid())
        .bind(&wallet.stx_address)
        .bind(&wallet.public_key)
        .bind(&wallet.encrypted_mnemonic)
        .bind(&wallet.kms_key_version)
        .bind(&wallet.network)
        .execute(&self.pool)
        .await
        .map_err(|err| AppError::Internal(err.into()))?;

        self.get_custodial_wallet(user_id)
            .await?
            .ok_or_else(|| AppError::Internal(anyhow::anyhow!("custodial wallet missing after insert")))
    }
}
