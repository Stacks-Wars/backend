use chrono::{DateTime, Utc};
use sqlx::PgPool;
use sw_domain::{User, UserId};
use uuid::Uuid;

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone)]
pub struct UpsertUserInput {
    pub id: UserId,
    pub email: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub email_verified_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateProfileInput {
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CustodialWalletInput {
    pub stx_address: String,
    pub public_key: String,
    pub encrypted_mnemonic: String,
    pub kms_key_version: String,
    pub network: String,
}

/// The subset of a profile that is safe to show to anyone.
#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct UserCard {
    pub id: Uuid,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
}

/// Public custodial wallet fields for transaction flows (no key material).
#[derive(Debug, Clone)]
pub struct CustodialWalletPublic {
    pub user_id: Uuid,
    pub stx_address: String,
    pub public_key: String,
    pub network: String,
}

/// Includes ciphertext for server-action signing (internal secret only).
#[derive(Debug, Clone)]
pub struct CustodialWalletSecret {
    pub user_id: Uuid,
    pub stx_address: String,
    pub public_key: String,
    pub network: String,
    pub encrypted_mnemonic: String,
    pub kms_key_version: String,
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

#[derive(Debug, sqlx::FromRow)]
struct CustodialWalletSecretRow {
    user_id: Uuid,
    stx_address: String,
    public_key: String,
    network: String,
    encrypted_mnemonic: String,
    kms_key_version: String,
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

impl From<CustodialWalletSecretRow> for CustodialWalletSecret {
    fn from(row: CustodialWalletSecretRow) -> Self {
        Self {
            user_id: row.user_id,
            stx_address: row.stx_address,
            public_key: row.public_key,
            network: row.network,
            encrypted_mnemonic: row.encrypted_mnemonic,
            kms_key_version: row.kms_key_version,
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

    pub async fn get_by_username(&self, username: &str) -> AppResult<Option<User>> {
        let row = sqlx::query_as::<_, UserRow>(
            r#"
            SELECT id, username, display_name, email, email_verified_at,
                   wallet_address, wallet_verified_at, avatar_url,
                   created_at, updated_at
            FROM users
            WHERE username = $1
            "#,
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| AppError::Internal(err.into()))?;

        Ok(row.map(User::from))
    }

    /// Partial profile update. `None` fields are left untouched.
    pub async fn update_profile(&self, id: UserId, input: UpdateProfileInput) -> AppResult<User> {
        if let Some(username) = input.username.as_deref() {
            let taken: bool = sqlx::query_scalar(
                r#"SELECT EXISTS(SELECT 1 FROM users WHERE username = $1 AND id <> $2)"#,
            )
            .bind(username)
            .bind(id.as_uuid())
            .fetch_one(&self.pool)
            .await
            .map_err(|err| AppError::Internal(err.into()))?;
            if taken {
                return Err(AppError::Conflict("username already taken".into()));
            }
        }

        let row = sqlx::query_as::<_, UserRow>(
            r#"
            UPDATE users SET
                username = COALESCE($2, username),
                display_name = COALESCE($3, display_name),
                avatar_url = COALESCE($4, avatar_url),
                updated_at = now()
            WHERE id = $1
            RETURNING id, username, display_name, email, email_verified_at,
                      wallet_address, wallet_verified_at, avatar_url,
                      created_at, updated_at
            "#,
        )
        .bind(id.as_uuid())
        .bind(&input.username)
        .bind(&input.display_name)
        .bind(&input.avatar_url)
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| AppError::Internal(err.into()))?;

        row.map(User::from).ok_or(AppError::NotFound("user"))
    }

    /// Resolve several users at once, for lists that show host and player names.
    pub async fn cards(&self, ids: &[Uuid]) -> AppResult<Vec<UserCard>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        sqlx::query_as::<_, UserCard>(
            r#"
            SELECT id, username, display_name, avatar_url
            FROM users
            WHERE id = ANY($1)
            "#,
        )
        .bind(ids)
        .fetch_all(&self.pool)
        .await
        .map_err(|err| AppError::Internal(err.into()))
    }

    pub async fn username_available(&self, username: &str) -> AppResult<bool> {
        let exists: bool =
            sqlx::query_scalar(r#"SELECT EXISTS(SELECT 1 FROM users WHERE username = $1)"#)
                .bind(username)
                .fetch_one(&self.pool)
                .await
                .map_err(|err| AppError::Internal(err.into()))?;
        Ok(!exists)
    }

    pub async fn upsert(&self, input: UpsertUserInput) -> AppResult<User> {
        // Email must not belong to a different user id.
        if let Some(existing) = sqlx::query_scalar::<_, Uuid>(
            r#"SELECT id FROM users WHERE email = $1 AND id <> $2 LIMIT 1"#,
        )
        .bind(&input.email)
        .bind(input.id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| AppError::Internal(err.into()))?
        {
            let _ = existing;
            return Err(AppError::Conflict(
                "email already registered to another account".into(),
            ));
        }

        let row = sqlx::query_as::<_, UserRow>(
            r#"
            INSERT INTO users (id, email, display_name, avatar_url, email_verified_at)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (id) DO UPDATE SET
                email = EXCLUDED.email,
                display_name = COALESCE(EXCLUDED.display_name, users.display_name),
                avatar_url = COALESCE(EXCLUDED.avatar_url, users.avatar_url),
                email_verified_at = COALESCE(EXCLUDED.email_verified_at, users.email_verified_at),
                updated_at = now()
            RETURNING id, username, display_name, email, email_verified_at,
                      wallet_address, wallet_verified_at, avatar_url,
                      created_at, updated_at
            "#,
        )
        .bind(input.id.as_uuid())
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

    pub async fn get_custodial_wallet_secret(
        &self,
        user_id: UserId,
    ) -> AppResult<Option<CustodialWalletSecret>> {
        let row = sqlx::query_as::<_, CustodialWalletSecretRow>(
            r#"
            SELECT user_id, stx_address, public_key, network,
                   encrypted_mnemonic, kms_key_version
            FROM custodial_wallets
            WHERE user_id = $1 AND status = 'active'
            LIMIT 1
            "#,
        )
        .bind(user_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| AppError::Internal(err.into()))?;

        Ok(row.map(CustodialWalletSecret::from))
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
