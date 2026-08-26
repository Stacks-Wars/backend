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
    async fn is_empty(&self) -> AppResult<bool>;
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

    /// If `seasons` is empty, insert Season 1..=N for Q1 through the current quarter of this year.
    pub async fn seed_year_to_current_quarter_if_empty(&self) -> AppResult<Vec<Season>> {
        if !self.is_empty().await? {
            return Ok(vec![]);
        }

        let now = Utc::now();
        let current = YearQuarter::of(now);
        let mut created = Vec::with_capacity(current.quarter as usize);

        for quarter in 1..=current.quarter {
            let yq = YearQuarter {
                year: current.year,
                quarter,
            };
            let (starts_at, ends_at) = yq.bounds();
            let season = self
                .create(CreateSeasonInput {
                    name: format!("Season {quarter}"),
                    description: None,
                    starts_at,
                    ends_at,
                })
                .await?;
            created.push(season);
        }

        info!(
            count = created.len(),
            year = current.year,
            through_quarter = current.quarter,
            "seeded quarterly seasons"
        );

        Ok(created)
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

    async fn is_empty(&self) -> AppResult<bool> {
        let count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*)::bigint FROM seasons")
            .fetch_one(&self.pool)
            .await
            .map_err(|err| AppError::Internal(err.into()))?;
        Ok(count == 0)
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
        .map_err(|err| AppError::Internal(err.into()))?;

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
}
