use chrono::{DateTime, Utc};
use sqlx::{Executor, PgPool, Postgres};
use sw_domain::{GameId, Lobby, LobbyId, LobbyStatus, UserId};
use uuid::Uuid;

use crate::data::lobby_status::DbLobbyStatus;
use crate::error::{AppError, AppResult, HostedLobbyRef};

/// A host may have this many unfinished (`waiting` / `starting` / `in_progress`) lobbies.
pub const MAX_ACTIVE_HOSTED_LOBBIES: usize = 2;

#[derive(Debug, sqlx::FromRow)]
struct LobbyRow {
    id: Uuid,
    path: String,
    name: String,
    description: Option<String>,
    game_id: String,
    creator_id: Uuid,
    chain: String,
    entry_amount_micro: i64,
    pot_micro: i64,
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
            chain: self.chain.parse().unwrap_or_default(),
            entry_amount_micro: self.entry_amount_micro,
            pot_micro: self.pot_micro,
            is_private: self.is_private,
            is_sponsored: self.is_sponsored,
            status: self.status.into(),
            created_at: self.created_at,
            updated_at: self.updated_at,
            participants: self.participants.into_iter().map(UserId::from).collect(),
        })
    }
}

/// Filters for the lobby browser. `None` means "any".
#[derive(Debug, Clone)]
pub struct LobbyQuery {
    pub game_id: Option<GameId>,
    pub statuses: Option<Vec<LobbyStatus>>,
    pub creator_id: Option<UserId>,
    /// `Some(true)` paid only, `Some(false)` free only.
    pub paid: Option<bool>,
    pub min_players: Option<i32>,
    pub max_players: Option<i32>,
    pub is_private: Option<bool>,
    pub chain: Option<String>,
    pub limit: i64,
    pub offset: i64,
}

impl Default for LobbyQuery {
    fn default() -> Self {
        Self {
            game_id: None,
            statuses: None,
            creator_id: None,
            paid: None,
            min_players: None,
            max_players: None,
            is_private: None,
            chain: None,
            limit: 60,
            offset: 0,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameActivity {
    pub game_id: String,
    pub waiting_lobbies: i64,
    pub live_lobbies: i64,
    pub active_players: i64,
    pub open_pot_micro: i64,
}

#[derive(Debug, sqlx::FromRow)]
struct GameActivityRow {
    game_id: String,
    waiting_lobbies: i64,
    live_lobbies: i64,
    active_players: i64,
    open_pot_micro: i64,
}

pub struct PgLobbyRepo {
    pool: PgPool,
}

impl PgLobbyRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn insert(&self, lobby: &Lobby) -> AppResult<()> {
        insert_lobby(&self.pool, lobby).await
    }

    /// Lock the creator, refuse a third unfinished host lobby, then insert.
    pub async fn insert_under_host_cap(&self, lobby: &Lobby) -> AppResult<()> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        lock_creator_xact(&mut tx, lobby.creator_id).await?;
        let active = list_active_created_by_exec(&mut *tx, lobby.creator_id).await?;
        if active.len() >= MAX_ACTIVE_HOSTED_LOBBIES {
            return Err(AppError::TooManyLobbies {
                lobbies: active.iter().map(hosted_lobby_ref).collect(),
            });
        }
        insert_lobby(&mut *tx, lobby).await?;
        tx.commit()
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        Ok(())
    }

    pub async fn get_by_path(&self, path: &str) -> AppResult<Option<Lobby>> {
        let row = sqlx::query_as::<_, LobbyRow>(
            r#"
            SELECT id, path, name, description, game_id, creator_id, chain::text AS chain,
                   entry_amount_micro, pot_micro,
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
            SELECT id, path, name, description, game_id, creator_id, chain::text AS chain,
                   entry_amount_micro, pot_micro,
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

    pub async fn list_open(&self, limit: i64, offset: i64) -> AppResult<Vec<Lobby>> {
        let rows = sqlx::query_as::<_, LobbyRow>(
            r#"
            SELECT id, path, name, description, game_id, creator_id, chain::text AS chain,
                   entry_amount_micro, pot_micro,
                   is_private, is_sponsored, status, participants,
                   created_at, updated_at
            FROM lobbies
            WHERE status IN ('waiting', 'starting', 'in_progress')
            ORDER BY created_at DESC
            LIMIT $1 OFFSET $2
            "#,
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::Internal(e.into()))?;

        rows.into_iter().map(LobbyRow::into_lobby).collect()
    }

    /// Browser query. `None` filters are ignored.
    pub async fn browse(&self, query: &LobbyQuery) -> AppResult<Vec<Lobby>> {
        let statuses: Option<Vec<String>> = query
            .statuses
            .as_ref()
            .map(|list| list.iter().map(|s| s.as_db_str().to_owned()).collect());

        let rows = sqlx::query_as::<_, LobbyRow>(
            r#"
            SELECT id, path, name, description, game_id, creator_id, chain::text AS chain,
                   entry_amount_micro, pot_micro,
                   is_private, is_sponsored, status, participants,
                   created_at, updated_at
            FROM lobbies
            WHERE ($1::BOOLEAN IS NULL OR is_private = $1)
              AND ($2::TEXT IS NULL OR game_id = $2)
              AND ($3::TEXT[] IS NULL OR status::TEXT = ANY($3))
              AND ($4::UUID IS NULL OR creator_id = $4)
              AND ($5::BOOLEAN IS NULL
                   OR ($5 = TRUE AND entry_amount_micro > 0)
                   OR ($5 = FALSE AND entry_amount_micro = 0))
              AND ($6::INT IS NULL OR cardinality(participants) >= $6)
              AND ($7::INT IS NULL OR cardinality(participants) <= $7)
              -- Paid/sponsored lobbies stay on their settlement chain.
              -- Free lobbies (entry 0) never hit a wallet, so they list everywhere.
              AND ($10::TEXT IS NULL
                   OR entry_amount_micro = 0
                   OR chain::text = $10)
            ORDER BY
                CASE status
                    WHEN 'waiting' THEN 0
                    WHEN 'starting' THEN 1
                    WHEN 'in_progress' THEN 2
                    ELSE 3
                END,
                created_at DESC
            LIMIT $8 OFFSET $9
            "#,
        )
        .bind(query.is_private)
        .bind(query.game_id.as_ref().map(GameId::as_str))
        .bind(statuses)
        .bind(query.creator_id.map(|c| c.as_uuid()))
        .bind(query.paid)
        .bind(query.min_players)
        .bind(query.max_players)
        .bind(query.limit)
        .bind(query.offset)
        .bind(query.chain.as_deref())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::Internal(e.into()))?;

        rows.into_iter().map(LobbyRow::into_lobby).collect()
    }

    /// Live lobby / player counts per game, for the games directory.
    pub async fn game_activity(&self) -> AppResult<Vec<GameActivity>> {
        let rows = sqlx::query_as::<_, GameActivityRow>(
            r#"
            SELECT game_id,
                   COUNT(*) FILTER (WHERE status = 'waiting') AS waiting_lobbies,
                   COUNT(*) FILTER (WHERE status IN ('starting', 'in_progress'))
                       AS live_lobbies,
                   COALESCE(SUM(cardinality(participants)), 0)::bigint AS active_players,
                   COALESCE(SUM(pot_micro), 0)::bigint AS open_pot_micro
            FROM lobbies
            WHERE status IN ('waiting', 'starting', 'in_progress')
            GROUP BY game_id
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::Internal(e.into()))?;

        Ok(rows
            .into_iter()
            .map(|r| GameActivity {
                game_id: r.game_id,
                waiting_lobbies: r.waiting_lobbies,
                live_lobbies: r.live_lobbies,
                active_players: r.active_players,
                open_pot_micro: r.open_pot_micro,
            })
            .collect())
    }

    /// Lobbies a user has taken part in, newest first.
    pub async fn list_for_participant(&self, user_id: UserId, limit: i64) -> AppResult<Vec<Lobby>> {
        let rows = sqlx::query_as::<_, LobbyRow>(
            r#"
            SELECT id, path, name, description, game_id, creator_id, chain::text AS chain,
                   entry_amount_micro, pot_micro,
                   is_private, is_sponsored, status, participants,
                   created_at, updated_at
            FROM lobbies
            WHERE $1 = ANY(participants)
            ORDER BY created_at DESC
            LIMIT $2
            "#,
        )
        .bind(user_id.as_uuid())
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::Internal(e.into()))?;

        rows.into_iter().map(LobbyRow::into_lobby).collect()
    }

    pub async fn path_exists(&self, path: &str) -> AppResult<bool> {
        let exists: bool =
            sqlx::query_scalar(r#"SELECT EXISTS(SELECT 1 FROM lobbies WHERE path = $1)"#)
                .bind(path)
                .fetch_one(&self.pool)
                .await
                .map_err(|e| AppError::Internal(e.into()))?;
        Ok(exists)
    }

    pub async fn add_participant(
        &self,
        id: LobbyId,
        user_id: UserId,
        entry_micro: i64,
    ) -> AppResult<()> {
        sqlx::query(
            r#"
            UPDATE lobbies
            SET participants = array_append(participants, $2),
                pot_micro = pot_micro + $3,
                updated_at = now()
            WHERE id = $1
              AND NOT ($2 = ANY(participants))
            "#,
        )
        .bind(id.as_uuid())
        .bind(user_id.as_uuid())
        .bind(entry_micro)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Internal(e.into()))?;
        Ok(())
    }

    pub async fn remove_participant(
        &self,
        id: LobbyId,
        user_id: UserId,
        entry_micro: i64,
    ) -> AppResult<()> {
        sqlx::query(
            r#"
            UPDATE lobbies
            SET participants = array_remove(participants, $2),
                pot_micro = GREATEST(pot_micro - $3, 0),
                updated_at = now()
            WHERE id = $1
            "#,
        )
        .bind(id.as_uuid())
        .bind(user_id.as_uuid())
        .bind(entry_micro)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Internal(e.into()))?;
        Ok(())
    }

    pub async fn set_status(&self, id: LobbyId, status: LobbyStatus) -> AppResult<()> {
        let status = DbLobbyStatus::from(status);
        sqlx::query(
            r#"
            UPDATE lobbies
            SET status = $2, updated_at = now()
            WHERE id = $1
            "#,
        )
        .bind(id.as_uuid())
        .bind(status)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Internal(e.into()))?;
        Ok(())
    }

    /// Waiting lobbies created before `cutoff` (used by the 24h TTL janitor).
    pub async fn list_waiting_older_than(&self, cutoff: DateTime<Utc>) -> AppResult<Vec<Lobby>> {
        let rows = sqlx::query_as::<_, LobbyRow>(
            r#"
            SELECT id, path, name, description, game_id, creator_id, chain::text AS chain,
                   entry_amount_micro, pot_micro,
                   is_private, is_sponsored, status, participants,
                   created_at, updated_at
            FROM lobbies
            WHERE status = 'waiting'
              AND created_at < $1
            ORDER BY created_at ASC
            LIMIT 100
            "#,
        )
        .bind(cutoff)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::Internal(e.into()))?;

        rows.into_iter().map(LobbyRow::into_lobby).collect()
    }

    pub async fn delete(&self, id: LobbyId) -> AppResult<()> {
        sqlx::query(r#"DELETE FROM lobbies WHERE id = $1"#)
            .bind(id.as_uuid())
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        Ok(())
    }

    pub async fn has_active_participation(&self, user_id: UserId) -> AppResult<bool> {
        let exists: bool = sqlx::query_scalar(
            r#"
            SELECT EXISTS(
                SELECT 1 FROM lobbies
                WHERE $1 = ANY(participants)
                  AND status IN ('waiting', 'starting', 'in_progress')
            )
            "#,
        )
        .bind(user_id.as_uuid())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::Internal(e.into()))?;
        Ok(exists)
    }

    pub async fn list_waiting_created_by(&self, user_id: UserId) -> AppResult<Vec<Lobby>> {
        let rows = sqlx::query_as::<_, LobbyRow>(
            r#"
            SELECT id, path, name, description, game_id, creator_id, chain::text AS chain,
                   entry_amount_micro, pot_micro,
                   is_private, is_sponsored, status, participants,
                   created_at, updated_at
            FROM lobbies
            WHERE creator_id = $1 AND status = 'waiting'
            "#,
        )
        .bind(user_id.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::Internal(e.into()))?;

        rows.into_iter().map(LobbyRow::into_lobby).collect()
    }

    pub async fn list_active_created_by(&self, user_id: UserId) -> AppResult<Vec<Lobby>> {
        list_active_created_by_exec(&self.pool, user_id).await
    }
}

pub fn hosted_lobby_ref(lobby: &Lobby) -> HostedLobbyRef {
    HostedLobbyRef {
        path: lobby.path.clone(),
        name: lobby.name.clone(),
        status: lobby.status,
    }
}

async fn lock_creator_xact(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    user_id: UserId,
) -> AppResult<()> {
    let uuid = user_id.as_uuid();
    let bytes = *uuid.as_bytes();
    let k1 = i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let k2 = i32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    sqlx::query("SELECT pg_advisory_xact_lock($1, $2)")
        .bind(k1)
        .bind(k2)
        .execute(&mut **tx)
        .await
        .map_err(|e| AppError::Internal(e.into()))?;
    Ok(())
}

async fn list_active_created_by_exec<'e, E>(exec: E, user_id: UserId) -> AppResult<Vec<Lobby>>
where
    E: Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query_as::<_, LobbyRow>(
        r#"
            SELECT id, path, name, description, game_id, creator_id, chain::text AS chain,
                   entry_amount_micro, pot_micro,
                   is_private, is_sponsored, status, participants,
                   created_at, updated_at
            FROM lobbies
            WHERE creator_id = $1
              AND status IN ('waiting', 'starting', 'in_progress')
            ORDER BY created_at ASC
            LIMIT 3
            "#,
    )
    .bind(user_id.as_uuid())
    .fetch_all(exec)
    .await
    .map_err(|e| AppError::Internal(e.into()))?;

    rows.into_iter().map(LobbyRow::into_lobby).collect()
}

async fn insert_lobby<'e, E>(exec: E, lobby: &Lobby) -> AppResult<()>
where
    E: Executor<'e, Database = Postgres>,
{
    let participant_uuids: Vec<Uuid> = lobby.participants.iter().map(|id| id.as_uuid()).collect();
    let status = DbLobbyStatus::from(lobby.status);

    sqlx::query(
        r#"
            INSERT INTO lobbies (
                id, path, name, description, game_id, creator_id, chain,
                entry_amount_micro, pot_micro,
                is_private, is_sponsored, status, participants,
                created_at, updated_at
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7::chain_id,
                $8, $9,
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
    .bind(lobby.chain.as_str())
    .bind(lobby.entry_amount_micro)
    .bind(lobby.pot_micro)
    .bind(lobby.is_private)
    .bind(lobby.is_sponsored)
    .bind(status)
    .bind(&participant_uuids)
    .bind(lobby.created_at)
    .bind(lobby.updated_at)
    .execute(exec)
    .await
    .map_err(|e| AppError::Internal(e.into()))?;

    Ok(())
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
