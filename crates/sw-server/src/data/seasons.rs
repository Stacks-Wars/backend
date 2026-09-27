use async_trait::async_trait;
use chrono::{DateTime, Datelike, Duration, TimeZone, Utc};
use sqlx::PgPool;
use sw_domain::{Season, SeasonId};
use tracing::info;

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone)]
pub struct CreateSeasonInput {
    pub name: String,
    pub description: Option<String>,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct UpdateSeasonInput {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct YearQuarter {
    pub year: i32,
    /// 1..=4
    pub quarter: u32,
}

impl YearQuarter {
    pub fn of(dt: DateTime<Utc>) -> Self {
        let month = dt.month();
        Self {
            year: dt.year(),
            quarter: ((month - 1) / 3) + 1,
        }
    }

    pub fn next(self) -> Self {
        if self.quarter == 4 {
            Self {
                year: self.year + 1,
                quarter: 1,
            }
        } else {
            Self {
                year: self.year,
                quarter: self.quarter + 1,
            }
        }
    }

    /// Inclusive window for the quarter (`starts_at`..=`ends_at`).
    pub fn bounds(self) -> (DateTime<Utc>, DateTime<Utc>) {
        let start_month = (self.quarter - 1) * 3 + 1;
        let starts_at = Utc
            .with_ymd_and_hms(self.year, start_month, 1, 0, 0, 0)
            .single()
            .expect("valid quarter start");

        let next = self.next();
        let next_start_month = (next.quarter - 1) * 3 + 1;
        let next_starts = Utc
            .with_ymd_and_hms(next.year, next_start_month, 1, 0, 0, 0)
            .single()
            .expect("valid next quarter start");

        // Inclusive end: one microsecond before the next quarter.
        let ends_at = next_starts - Duration::microseconds(1);
        (starts_at, ends_at)
    }
}

/// Whether the season ending at `ends_at` should get its successor now.
///
/// Split out from the repo so the lead window is testable without a database.
/// A negative remaining time (the boundary already passed, for example while the
/// cron was down) also counts as due, which is what makes a missed run heal.
pub fn successor_due(ends_at: DateTime<Utc>, now: DateTime<Utc>, lead: Duration) -> bool {
    ends_at - now <= lead
}

/// Postgres `unique_violation`. Used to turn the `seasons_window_unique` index
/// backstop into a 409 instead of a 500.
fn is_unique_violation(err: &sqlx::Error) -> bool {
    err.as_database_error()
        .and_then(|db| db.code())
        .is_some_and(|code| code == "23505")
}

fn normalize_name(name: &str) -> AppResult<String> {
    let name = name.trim().to_owned();
    if name.is_empty() || name.len() > 120 {
        return Err(AppError::BadRequest("name must be 1–120 characters".into()));
    }
    Ok(name)
}

fn normalize_description(description: Option<String>) -> Option<String> {
    description.and_then(|d| {
        let trimmed = d.trim().to_owned();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

#[derive(Debug, sqlx::FromRow)]
struct SeasonRow {
    id: i32,
    name: String,
    description: Option<String>,
    starts_at: DateTime<Utc>,
    ends_at: DateTime<Utc>,
    created_at: DateTime<Utc>,
}

impl From<SeasonRow> for Season {
    fn from(row: SeasonRow) -> Self {
        Self {
            id: SeasonId(row.id),
            name: row.name,
            description: row.description,
            starts_at: row.starts_at,
            ends_at: row.ends_at,
            created_at: row.created_at,
        }
    }
}

#[async_trait]
pub trait SeasonRepo: Send + Sync {
    async fn current(&self) -> AppResult<Option<Season>>;
    async fn get(&self, id: SeasonId) -> AppResult<Option<Season>>;
    async fn list(&self, limit: i64, offset: i64) -> AppResult<Vec<Season>>;
    async fn latest(&self) -> AppResult<Option<Season>>;
    async fn create(&self, input: CreateSeasonInput) -> AppResult<Season>;
    async fn update(&self, id: SeasonId, input: UpdateSeasonInput) -> AppResult<Season>;
}

pub struct PgSeasonRepo {
    pool: PgPool,
}

impl PgSeasonRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Create the next quarterly season after the latest one (or the current quarter if empty).
    pub async fn create_next_quarter(
        &self,
        name: String,
        description: Option<String>,
    ) -> AppResult<Season> {
        let name = normalize_name(&name)?;
        let description = normalize_description(description);

        let yq = match self.latest().await? {
            Some(latest) => YearQuarter::of(latest.starts_at).next(),
            None => YearQuarter::of(Utc::now()),
        };
        let (starts_at, ends_at) = yq.bounds();

        self.create(CreateSeasonInput {
            name,
            description,
            starts_at,
            ends_at,
        })
        .await
    }

    /// Create the successor season when the latest one is within `lead` of its
    /// end, and only then. Returns `None` when there is nothing to do, which is
    /// every run except the few hours before a quarter boundary — so at most one
    /// future season is ever on the list.
    ///
    /// Idempotent: the successor's window is derived from the latest season, and
    /// an existing row for that window short-circuits.
    pub async fn ensure_upcoming_season(&self, lead: Duration) -> AppResult<Option<Season>> {
        let now = Utc::now();
        let Some(latest) = self.latest().await? else {
            // Empty table (fresh database): bootstrap the quarter we are in.
            let yq = YearQuarter::of(now);
            let (starts_at, ends_at) = yq.bounds();
            return self
                .create_season_if_absent(starts_at, ends_at)
                .await
                .map(Some);
        };

        let yq = YearQuarter::of(latest.starts_at).next();
        let (starts_at, ends_at) = yq.bounds();
        if self.season_starts_at(starts_at).await? {
            return Ok(None);
        }
        if !successor_due(latest.ends_at, now, lead) {
            return Ok(None);
        }

        self.create_season_if_absent(starts_at, ends_at)
            .await
            .map(Some)
    }

    /// Insert `Season {n}` for the given window, numbering past the highest
    /// existing season so the sequence never restarts with the calendar year.
    /// Concurrent runs lose the unique-index race and return the existing row.
    async fn create_season_if_absent(
        &self,
        starts_at: DateTime<Utc>,
        ends_at: DateTime<Utc>,
    ) -> AppResult<Season> {
        for _ in 0..64 {
            let name = self.next_season_name().await?;
            let row = sqlx::query_as::<_, SeasonRow>(
                r#"
                INSERT INTO seasons (name, description, starts_at, ends_at)
                VALUES ($1, $2, $3, $4)
                ON CONFLICT (starts_at, ends_at) DO NOTHING
                RETURNING id, name, description, starts_at, ends_at, created_at
                "#,
            )
            .bind(&name)
            .bind(Option::<String>::None)
            .bind(starts_at)
            .bind(ends_at)
            .fetch_optional(&self.pool)
            .await
            .map_err(|err| AppError::Internal(err.into()))?;

            if let Some(row) = row {
                let season = Season::from(row);
                info!(
                    season_id = season.id.0,
                    name = %season.name,
                    starts_at = %season.starts_at,
                    "created upcoming season"
                );
                return Ok(season);
            }

            // Window exists: either another run won, or the name was taken.
            if let Some(existing) = self.season_by_window(starts_at, ends_at).await? {
                return Ok(existing);
            }
        }
        Err(AppError::Internal(anyhow::anyhow!(
            "could not allocate a season name"
        )))
    }

    async fn next_season_name(&self) -> AppResult<String> {
        let count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*)::bigint FROM seasons")
            .fetch_one(&self.pool)
            .await
            .map_err(|err| AppError::Internal(err.into()))?;

        let mut number = count + 1;
        while self.season_name_taken(number).await? {
            number += 1;
        }
        Ok(format!("Season {number}"))
    }

    async fn season_name_taken(&self, number: i64) -> AppResult<bool> {
        let exists = sqlx::query_scalar::<_, bool>(
            r#"SELECT EXISTS(SELECT 1 FROM seasons WHERE name = $1)"#,
        )
        .bind(format!("Season {number}"))
        .fetch_one(&self.pool)
        .await
        .map_err(|err| AppError::Internal(err.into()))?;
        Ok(exists)
    }

    async fn season_starts_at(&self, starts_at: DateTime<Utc>) -> AppResult<bool> {
        let exists = sqlx::query_scalar::<_, bool>(
            r#"SELECT EXISTS(SELECT 1 FROM seasons WHERE starts_at = $1)"#,
        )
        .bind(starts_at)
        .fetch_one(&self.pool)
        .await
        .map_err(|err| AppError::Internal(err.into()))?;
        Ok(exists)
    }

    async fn season_by_window(
        &self,
        starts_at: DateTime<Utc>,
        ends_at: DateTime<Utc>,
    ) -> AppResult<Option<Season>> {
        let row = sqlx::query_as::<_, SeasonRow>(
            r#"
            SELECT id, name, description, starts_at, ends_at, created_at
            FROM seasons
            WHERE starts_at = $1 AND ends_at = $2
            "#,
        )
        .bind(starts_at)
        .bind(ends_at)
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| AppError::Internal(err.into()))?;

        Ok(row.map(Season::from))
    }
}

#[async_trait]
impl SeasonRepo for PgSeasonRepo {
    async fn current(&self) -> AppResult<Option<Season>> {
        let now = Utc::now();
        let row = sqlx::query_as::<_, SeasonRow>(
            r#"
            SELECT id, name, description, starts_at, ends_at, created_at
            FROM seasons
            WHERE starts_at <= $1 AND ends_at >= $1
            ORDER BY starts_at DESC
            LIMIT 1
            "#,
        )
        .bind(now)
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| AppError::Internal(err.into()))?;

        Ok(row.map(Season::from))
    }

    async fn get(&self, id: SeasonId) -> AppResult<Option<Season>> {
        let row = sqlx::query_as::<_, SeasonRow>(
            r#"
            SELECT id, name, description, starts_at, ends_at, created_at
            FROM seasons
            WHERE id = $1
            "#,
        )
        .bind(id.as_i32())
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| AppError::Internal(err.into()))?;

        Ok(row.map(Season::from))
    }

    async fn list(&self, limit: i64, offset: i64) -> AppResult<Vec<Season>> {
        let rows = sqlx::query_as::<_, SeasonRow>(
            r#"
            SELECT id, name, description, starts_at, ends_at, created_at
            FROM seasons
            ORDER BY starts_at DESC
            LIMIT $1 OFFSET $2
            "#,
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .map_err(|err| AppError::Internal(err.into()))?;

        Ok(rows.into_iter().map(Season::from).collect())
    }

    async fn latest(&self) -> AppResult<Option<Season>> {
        let row = sqlx::query_as::<_, SeasonRow>(
            r#"
            SELECT id, name, description, starts_at, ends_at, created_at
            FROM seasons
            ORDER BY starts_at DESC
            LIMIT 1
            "#,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| AppError::Internal(err.into()))?;

        Ok(row.map(Season::from))
    }

    async fn create(&self, input: CreateSeasonInput) -> AppResult<Season> {
        if input.ends_at <= input.starts_at {
            return Err(AppError::BadRequest("endsAt must be after startsAt".into()));
        }
        let name = normalize_name(&input.name)?;
        let description = normalize_description(input.description);

        let row = sqlx::query_as::<_, SeasonRow>(
            r#"
            INSERT INTO seasons (name, description, starts_at, ends_at)
            VALUES ($1, $2, $3, $4)
            RETURNING id, name, description, starts_at, ends_at, created_at
            "#,
        )
        .bind(name)
        .bind(description)
        .bind(input.starts_at)
        .bind(input.ends_at)
        .fetch_one(&self.pool)
        .await
        .map_err(|err| {
            // `seasons_window_unique`: the window already exists. The scheduled
            // job creates the upcoming season before the boundary, so an admin
            // creating the same quarter by hand is a conflict, not a failure.
            if is_unique_violation(&err) {
                AppError::Conflict("a season already covers that window".into())
            } else {
                AppError::Internal(err.into())
            }
        })?;

        Ok(Season::from(row))
    }

    async fn update(&self, id: SeasonId, input: UpdateSeasonInput) -> AppResult<Season> {
        let name = normalize_name(&input.name)?;
        let description = normalize_description(input.description);

        let row = sqlx::query_as::<_, SeasonRow>(
            r#"
            UPDATE seasons
            SET name = $2, description = $3
            WHERE id = $1
            RETURNING id, name, description, starts_at, ends_at, created_at
            "#,
        )
        .bind(id.as_i32())
        .bind(name)
        .bind(description)
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| AppError::Internal(err.into()))?
        .ok_or(AppError::NotFound("season not found"))?;

        Ok(Season::from(row))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quarter_of_july_is_q3() {
        let dt = Utc.with_ymd_and_hms(2026, 7, 28, 12, 0, 0).unwrap();
        assert_eq!(
            YearQuarter::of(dt),
            YearQuarter {
                year: 2026,
                quarter: 3
            }
        );
    }

    #[test]
    fn q3_bounds() {
        let (start, end) = YearQuarter {
            year: 2026,
            quarter: 3,
        }
        .bounds();
        assert_eq!(start, Utc.with_ymd_and_hms(2026, 7, 1, 0, 0, 0).unwrap());
        let q4_start = Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap();
        assert_eq!(end, q4_start - Duration::microseconds(1));
    }

    #[test]
    fn q4_rolls_into_next_year() {
        let q4 = YearQuarter {
            year: 2026,
            quarter: 4,
        };
        assert_eq!(
            q4.next(),
            YearQuarter {
                year: 2027,
                quarter: 1
            }
        );
    }

    /// The boundary the cron watches: Q4 ends a microsecond before Jan 1.
    #[test]
    fn q4_bounds_stop_before_new_year() {
        let (start, end) = YearQuarter {
            year: 2026,
            quarter: 4,
        }
        .bounds();
        assert_eq!(start, Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap());
        let q1_2027 = Utc.with_ymd_and_hms(2027, 1, 1, 0, 0, 0).unwrap();
        assert_eq!(end, q1_2027 - Duration::microseconds(1));
    }

    /// Successor windows must line up: Q3 ends where Q4 begins.
    #[test]
    fn quarter_windows_are_contiguous() {
        let q3 = YearQuarter {
            year: 2026,
            quarter: 3,
        };
        let (_, q3_end) = q3.bounds();
        let (q4_start, _) = q3.next().bounds();
        assert_eq!(q4_start - q3_end, Duration::microseconds(1));
    }

    const LEAD: Duration = Duration::hours(6);

    #[test]
    fn successor_not_due_while_the_season_has_time_left() {
        let now = Utc.with_ymd_and_hms(2026, 9, 10, 0, 0, 0).unwrap();
        let ends_at = Utc.with_ymd_and_hms(2026, 9, 30, 23, 59, 59).unwrap();
        assert!(!successor_due(ends_at, now, LEAD));
    }

    #[test]
    fn successor_due_inside_the_lead_window() {
        let now = Utc.with_ymd_and_hms(2026, 9, 30, 18, 0, 0).unwrap();
        let ends_at = Utc.with_ymd_and_hms(2026, 9, 30, 23, 59, 59).unwrap();
        assert!(successor_due(ends_at, now, LEAD));
    }

    #[test]
    fn successor_due_exactly_at_the_lead_edge() {
        let ends_at = Utc.with_ymd_and_hms(2026, 9, 30, 23, 0, 0).unwrap();
        let now = ends_at - LEAD;
        assert!(successor_due(ends_at, now, LEAD));
    }

    /// A run after the boundary still creates the season, so a skipped run
    /// during the lead window is recoverable.
    #[test]
    fn successor_due_when_the_boundary_already_passed() {
        let ends_at = Utc.with_ymd_and_hms(2026, 9, 30, 23, 59, 59).unwrap();
        let now = Utc.with_ymd_and_hms(2026, 10, 1, 4, 0, 0).unwrap();
        assert!(successor_due(ends_at, now, LEAD));
    }
}
