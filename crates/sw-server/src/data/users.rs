use chrono::{DateTime, Utc};
use sqlx::PgPool;
use sw_domain::{ChainId, User, UserId};
use uuid::Uuid;

use crate::error::{AppError, AppResult};

/// KMS version 1 / `local:dev1` has no AAD. Version 2+ does.
pub fn kms_key_uses_aad(kms_key_version: &str) -> bool {
    kms_wrap_version(kms_key_version) >= 2
}

fn kms_wrap_version(kms_key_version: &str) -> u32 {
    if let Some(rest) = kms_key_version.rsplit_once("/cryptoKeyVersions/") {
        return rest.1.parse().unwrap_or(1);
    }
    if let Some(rest) = kms_key_version.strip_prefix("local:dev") {
        return rest.parse().unwrap_or(1);
    }
    1
}

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
    pub address: String,
    pub public_key: String,
    pub encrypted_signing_material: String,
    pub kms_key_version: String,
    pub network: String,
    pub chain: String,
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
    pub address: String,
    pub public_key: String,
    pub network: String,
    pub chain: String,
}

/// Includes ciphertext for server-action signing (internal secret only).
#[derive(Debug, Clone)]
pub struct CustodialWalletSecret {
    pub id: Uuid,
    pub user_id: Uuid,
    pub address: String,
    pub public_key: String,
    pub network: String,
    pub chain: String,
    pub encrypted_signing_material: String,
    pub kms_key_version: String,
}

#[derive(Debug, sqlx::FromRow)]
#[allow(dead_code)]
struct UserRow {
    id: Uuid,
    username: Option<String>,
    display_name: Option<String>,
    email: String,
    email_verified_at: Option<DateTime<Utc>>,
    avatar_url: Option<String>,
    lobby_alerts_enabled: bool,
    legal_accepted_at: Option<DateTime<Utc>>,
    legal_version: Option<String>,
    deleted_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, sqlx::FromRow)]
struct CustodialWalletRow {
    user_id: Uuid,
    address: String,
    public_key: String,
    network: String,
    chain: String,
}

#[derive(Debug, sqlx::FromRow)]
struct CustodialWalletSecretRow {
    id: Uuid,
    user_id: Uuid,
    address: String,
    public_key: String,
    network: String,
    chain: String,
    encrypted_signing_material: String,
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
            address: row.address,
            public_key: row.public_key,
            network: row.network,
            chain: row.chain,
        }
    }
}

impl From<CustodialWalletSecretRow> for CustodialWalletSecret {
    fn from(row: CustodialWalletSecretRow) -> Self {
        Self {
            id: row.id,
            user_id: row.user_id,
            address: row.address,
            public_key: row.public_key,
            network: row.network,
            chain: row.chain,
            encrypted_signing_material: row.encrypted_signing_material,
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
                   avatar_url,
                   lobby_alerts_enabled, legal_accepted_at, legal_version,
                   deleted_at, created_at, updated_at
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

    pub async fn get_active_by_id(&self, id: UserId) -> AppResult<Option<User>> {
        let row = sqlx::query_as::<_, UserRow>(
            r#"
            SELECT id, username, display_name, email, email_verified_at,
                   avatar_url,
                   lobby_alerts_enabled, legal_accepted_at, legal_version,
                   deleted_at, created_at, updated_at
            FROM users
            WHERE id = $1 AND deleted_at IS NULL
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
                   avatar_url,
                   lobby_alerts_enabled, legal_accepted_at, legal_version,
                   deleted_at, created_at, updated_at
            FROM users
            WHERE username = $1 AND deleted_at IS NULL
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
                      avatar_url,
                      lobby_alerts_enabled, legal_accepted_at, legal_version,
                      deleted_at, created_at, updated_at
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
                      avatar_url,
                      lobby_alerts_enabled, legal_accepted_at, legal_version,
                      deleted_at, created_at, updated_at
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

        if row.deleted_at.is_some() {
            return Err(AppError::Unauthorized("account deleted"));
        }

        Ok(User::from(row))
    }

    pub async fn get_custodial_wallet(
        &self,
        user_id: UserId,
        chain: &str,
    ) -> AppResult<Option<CustodialWalletPublic>> {
        let row = sqlx::query_as::<_, CustodialWalletRow>(
            r#"
            SELECT user_id, address, public_key, network, chain::text AS chain
            FROM custodial_wallets
            WHERE user_id = $1 AND chain::text = $2 AND status = 'active'
            LIMIT 1
            "#,
        )
        .bind(user_id.as_uuid())
        .bind(chain)
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| AppError::Internal(err.into()))?;

        Ok(row
            .map(CustodialWalletPublic::from)
            .filter(|w| ChainId::from_optional(Some(&w.chain)).matches_address(&w.address)))
    }

    pub async fn list_custodial_wallets(
        &self,
        user_id: UserId,
    ) -> AppResult<Vec<CustodialWalletPublic>> {
        let rows = sqlx::query_as::<_, CustodialWalletRow>(
            r#"
            SELECT user_id, address, public_key, network, chain::text AS chain
            FROM custodial_wallets
            WHERE user_id = $1 AND status = 'active'
            ORDER BY CASE chain::text WHEN 'solana' THEN 0 ELSE 1 END, created_at ASC
            "#,
        )
        .bind(user_id.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(|err| AppError::Internal(err.into()))?;

        Ok(rows
            .into_iter()
            .map(CustodialWalletPublic::from)
            .filter(|w| ChainId::from_optional(Some(&w.chain)).matches_address(&w.address))
            .collect())
    }

    pub async fn get_custodial_wallet_secret(
        &self,
        user_id: UserId,
        chain: &str,
    ) -> AppResult<Option<CustodialWalletSecret>> {
        let row = sqlx::query_as::<_, CustodialWalletSecretRow>(
            r#"
            SELECT id, user_id, address, public_key, network, chain::text AS chain,
                   encrypted_signing_material, kms_key_version
            FROM custodial_wallets
            WHERE user_id = $1 AND chain::text = $2 AND status = 'active'
            LIMIT 1
            "#,
        )
        .bind(user_id.as_uuid())
        .bind(chain)
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| AppError::Internal(err.into()))?;

        Ok(row
            .map(CustodialWalletSecret::from)
            .filter(|w| ChainId::from_optional(Some(&w.chain)).matches_address(&w.address)))
    }

    pub async fn create_custodial_wallet(
        &self,
        user_id: UserId,
        wallet: CustodialWalletInput,
    ) -> AppResult<CustodialWalletPublic> {
        if self.get_active_by_id(user_id).await?.is_none() {
            return Err(AppError::NotFound("user"));
        }

        self.relabel_stacks_addresses(user_id).await?;

        if let Some(existing) = self.get_custodial_wallet(user_id, &wallet.chain).await? {
            return Ok(existing);
        }

        sqlx::query(
            r#"
            INSERT INTO custodial_wallets (
                user_id, address, public_key, encrypted_signing_material,
                kms_key_version, network, chain
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7::chain_id)
            ON CONFLICT (user_id, chain, network) DO NOTHING
            "#,
        )
        .bind(user_id.as_uuid())
        .bind(&wallet.address)
        .bind(&wallet.public_key)
        .bind(&wallet.encrypted_signing_material)
        .bind(&wallet.kms_key_version)
        .bind(&wallet.network)
        .bind(&wallet.chain)
        .execute(&self.pool)
        .await
        .map_err(|err| AppError::Internal(err.into()))?;

        self.get_custodial_wallet(user_id, &wallet.chain)
            .await?
            .ok_or_else(|| {
                AppError::Internal(anyhow::anyhow!("custodial wallet missing after insert"))
            })
    }

    /// Rows that look like Stacks principals must not sit on `chain = solana`.
    /// That happened when ADD COLUMN defaulted existing wallets to solana.
    async fn relabel_stacks_addresses(&self, user_id: UserId) -> AppResult<()> {
        sqlx::query(
            r#"
            UPDATE custodial_wallets
            SET chain = 'stacks'
            WHERE user_id = $1
              AND chain = 'solana'
              AND (address LIKE 'SP%' OR address LIKE 'ST%')
              AND address !~ '[a-z]'
            "#,
        )
        .bind(user_id.as_uuid())
        .execute(&self.pool)
        .await
        .map_err(|err| AppError::Internal(err.into()))?;
        Ok(())
    }

    pub async fn update_custodial_wallet_encryption(
        &self,
        user_id: UserId,
        chain: &str,
        encrypted_signing_material: &str,
        kms_key_version: &str,
    ) -> AppResult<bool> {
        let result = sqlx::query(
            r#"
            UPDATE custodial_wallets
            SET encrypted_signing_material = $2,
                kms_key_version = $3,
                updated_at = now()
            WHERE user_id = $1
              AND chain::text = $4
              AND status = 'active'
              AND (
                    kms_key_version ~ '/cryptoKeyVersions/1$'
                    OR kms_key_version = 'local:dev1'
                  )
            "#,
        )
        .bind(user_id.as_uuid())
        .bind(encrypted_signing_material)
        .bind(kms_key_version)
        .bind(chain)
        .execute(&self.pool)
        .await
        .map_err(|err| AppError::Internal(err.into()))?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn prefs(&self, id: UserId) -> AppResult<Option<UserPrefs>> {
        let row = sqlx::query_as::<_, UserPrefsRow>(
            r#"
            SELECT lobby_alerts_enabled, current_chain::text AS current_chain,
                   legal_accepted_at, legal_version, deleted_at
            FROM users
            WHERE id = $1
            "#,
        )
        .bind(id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| AppError::Internal(err.into()))?;

        Ok(row.map(|r| UserPrefs {
            lobby_alerts_enabled: r.lobby_alerts_enabled,
            current_chain: r.current_chain.parse().unwrap_or_default(),
            legal_accepted_at: r.legal_accepted_at,
            legal_version: r.legal_version,
            deleted_at: r.deleted_at,
        }))
    }

    pub async fn accept_legal(&self, id: UserId, version: &str) -> AppResult<()> {
        let n = sqlx::query(
            r#"
            UPDATE users SET
                legal_accepted_at = now(),
                legal_version = $2,
                updated_at = now()
            WHERE id = $1 AND deleted_at IS NULL
            "#,
        )
        .bind(id.as_uuid())
        .bind(version)
        .execute(&self.pool)
        .await
        .map_err(|err| AppError::Internal(err.into()))?
        .rows_affected();
        if n == 0 {
            return Err(AppError::NotFound("user"));
        }
        Ok(())
    }

    pub async fn set_lobby_alerts(&self, id: UserId, enabled: bool) -> AppResult<()> {
        let n = sqlx::query(
            r#"
            UPDATE users SET lobby_alerts_enabled = $2, updated_at = now()
            WHERE id = $1 AND deleted_at IS NULL
            "#,
        )
        .bind(id.as_uuid())
        .bind(enabled)
        .execute(&self.pool)
        .await
        .map_err(|err| AppError::Internal(err.into()))?
        .rows_affected();
        if n == 0 {
            return Err(AppError::NotFound("user"));
        }
        Ok(())
    }

    pub async fn set_current_chain(&self, id: UserId, chain: ChainId) -> AppResult<()> {
        let n = sqlx::query(
            r#"
            UPDATE users SET current_chain = $2::chain_id, updated_at = now()
            WHERE id = $1 AND deleted_at IS NULL
            "#,
        )
        .bind(id.as_uuid())
        .bind(chain.as_str())
        .execute(&self.pool)
        .await
        .map_err(|err| AppError::Internal(err.into()))?
        .rows_affected();
        if n == 0 {
            return Err(AppError::NotFound("user"));
        }
        Ok(())
    }

    pub async fn quest_flags(&self, id: UserId) -> AppResult<Option<QuestFlags>> {
        Self::quest_flags_on(&self.pool, id).await
    }

    pub async fn quest_flags_on<'e, E>(exec: E, id: UserId) -> AppResult<Option<QuestFlags>>
    where
        E: sqlx::Executor<'e, Database = sqlx::Postgres>,
    {
        sqlx::query_as::<_, QuestFlags>(
            r#"
            SELECT
                (username IS NOT NULL) AS username_set,
                referral_prompt_status::text AS referral_prompt_status,
                quest_intro_seen_at,
                getting_started_completed_at
            FROM users
            WHERE id = $1 AND deleted_at IS NULL
            "#,
        )
        .bind(id.as_uuid())
        .fetch_optional(exec)
        .await
        .map_err(|err| AppError::Internal(err.into()))
    }

    /// Write-once: pending → set. Returns false if the prompt was already answered.
    pub async fn set_referral(&self, id: UserId, referrer_id: UserId) -> AppResult<bool> {
        let n = sqlx::query(
            r#"
            UPDATE users SET
                referred_by_user_id = $2,
                referred_at = now(),
                referral_prompt_status = 'set',
                updated_at = now()
            WHERE id = $1
              AND deleted_at IS NULL
              AND referral_prompt_status = 'pending'
              AND id <> $2
            "#,
        )
        .bind(id.as_uuid())
        .bind(referrer_id.as_uuid())
        .execute(&self.pool)
        .await
        .map_err(|err| AppError::Internal(err.into()))?
        .rows_affected();
        Ok(n > 0)
    }

    /// Write-once: pending → skipped.
    pub async fn skip_referral(&self, id: UserId) -> AppResult<bool> {
        let n = sqlx::query(
            r#"
            UPDATE users SET
                referral_prompt_status = 'skipped',
                updated_at = now()
            WHERE id = $1
              AND deleted_at IS NULL
              AND referral_prompt_status = 'pending'
            "#,
        )
        .bind(id.as_uuid())
        .execute(&self.pool)
        .await
        .map_err(|err| AppError::Internal(err.into()))?
        .rows_affected();
        Ok(n > 0)
    }

    pub async fn mark_quest_intro_seen(&self, id: UserId) -> AppResult<()> {
        let n = sqlx::query(
            r#"
            UPDATE users SET
                quest_intro_seen_at = COALESCE(quest_intro_seen_at, now()),
                updated_at = now()
            WHERE id = $1 AND deleted_at IS NULL
            "#,
        )
        .bind(id.as_uuid())
        .execute(&self.pool)
        .await
        .map_err(|err| AppError::Internal(err.into()))?
        .rows_affected();
        if n == 0 {
            return Err(AppError::NotFound("user"));
        }
        Ok(())
    }

    /// Scrub PII, drop custodial keys, and remove the Neon Auth identity so the
    /// email can be used to sign up again. Keeps the `users` row for FK history.
    pub async fn anonymize(&self, id: UserId) -> AppResult<()> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|err| AppError::Internal(err.into()))?;

        let tombstone = format!("deleted+{}@invalid", id.as_uuid());
        let neon_email: Option<String> =
            sqlx::query_scalar(r#"SELECT email FROM neon_auth."user" WHERE id = $1"#)
                .bind(id.as_uuid())
                .fetch_optional(&mut *tx)
                .await
                .map_err(|err| AppError::Internal(err.into()))?;

        sqlx::query(
            r#"
            UPDATE users SET
                username = NULL,
                display_name = 'Deleted player',
                email = $2,
                email_verified_at = NULL,
                avatar_url = NULL,
                lobby_alerts_enabled = false,
                referred_by_user_id = NULL,
                deleted_at = now(),
                updated_at = now()
            WHERE id = $1 AND deleted_at IS NULL
            "#,
        )
        .bind(id.as_uuid())
        .bind(&tombstone)
        .execute(&mut *tx)
        .await
        .map_err(|err| AppError::Internal(err.into()))?;

        sqlx::query(
            r#"UPDATE users SET referred_by_user_id = NULL WHERE referred_by_user_id = $1"#,
        )
        .bind(id.as_uuid())
        .execute(&mut *tx)
        .await
        .map_err(|err| AppError::Internal(err.into()))?;

        sqlx::query(r#"DELETE FROM custodial_wallets WHERE user_id = $1"#)
            .bind(id.as_uuid())
            .execute(&mut *tx)
            .await
            .map_err(|err| AppError::Internal(err.into()))?;

        sqlx::query(r#"DELETE FROM user_game_stats WHERE user_id = $1"#)
            .bind(id.as_uuid())
            .execute(&mut *tx)
            .await
            .map_err(|err| AppError::Internal(err.into()))?;

        if let Some(email) = neon_email.as_deref() {
            sqlx::query(r#"DELETE FROM neon_auth.verification WHERE identifier = $1"#)
                .bind(email)
                .execute(&mut *tx)
                .await
                .map_err(|err| AppError::Internal(err.into()))?;
        }

        // Frees `neon_auth.user.email` (unique). Sessions/accounts cascade.
        sqlx::query(r#"DELETE FROM neon_auth."user" WHERE id = $1"#)
            .bind(id.as_uuid())
            .execute(&mut *tx)
            .await
            .map_err(|err| AppError::Internal(err.into()))?;

        tx.commit()
            .await
            .map_err(|err| AppError::Internal(err.into()))?;
        Ok(())
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct UserPrefsRow {
    lobby_alerts_enabled: bool,
    current_chain: String,
    legal_accepted_at: Option<DateTime<Utc>>,
    legal_version: Option<String>,
    deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct UserPrefs {
    pub lobby_alerts_enabled: bool,
    pub current_chain: ChainId,
    pub legal_accepted_at: Option<DateTime<Utc>>,
    pub legal_version: Option<String>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct QuestFlags {
    pub username_set: bool,
    pub referral_prompt_status: String,
    pub quest_intro_seen_at: Option<DateTime<Utc>>,
    pub getting_started_completed_at: Option<DateTime<Utc>>,
}
