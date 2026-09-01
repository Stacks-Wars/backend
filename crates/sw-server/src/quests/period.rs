//! UTC quest periods. Client-supplied dates are never trusted.

use chrono::{DateTime, Datelike, Duration, TimeZone, Utc};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PeriodKind {
    GettingStarted,
    Daily,
    Weekly,
    Monthly,
    Seasonal,
    PaidLadder,
}

impl PeriodKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::GettingStarted => "getting_started",
            Self::Daily => "daily",
            Self::Weekly => "weekly",
            Self::Monthly => "monthly",
            Self::Seasonal => "seasonal",
            Self::PaidLadder => "paid_ladder",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeriodClock {
    pub kind: PeriodKind,
    pub id: String,
    pub starts_at: DateTime<Utc>,
    pub resets_at: Option<DateTime<Utc>>,
}

pub fn daily(now: DateTime<Utc>) -> PeriodClock {
    let day = now.date_naive();
    let starts_at = Utc.from_utc_datetime(&day.and_hms_opt(0, 0, 0).expect("midnight"));
    let resets_at = starts_at + Duration::days(1);
    PeriodClock {
        kind: PeriodKind::Daily,
        id: day.format("%Y-%m-%d").to_string(),
        starts_at,
        resets_at: Some(resets_at),
    }
}

pub fn weekly(now: DateTime<Utc>) -> PeriodClock {
    let iso = now.iso_week();
    let id = format!("{}-W{:02}", iso.year(), iso.week());
    let weekday = now.weekday().num_days_from_monday() as i64;
    let day = now.date_naive() - Duration::days(weekday);
    let starts_at = Utc.from_utc_datetime(&day.and_hms_opt(0, 0, 0).expect("midnight"));
    PeriodClock {
        kind: PeriodKind::Weekly,
        id,
        starts_at,
        resets_at: Some(starts_at + Duration::weeks(1)),
    }
}

pub fn monthly(now: DateTime<Utc>) -> PeriodClock {
    let starts_at = Utc
        .with_ymd_and_hms(now.year(), now.month(), 1, 0, 0, 0)
        .single()
        .expect("month start");
    let next = if now.month() == 12 {
        Utc.with_ymd_and_hms(now.year() + 1, 1, 1, 0, 0, 0)
    } else {
        Utc.with_ymd_and_hms(now.year(), now.month() + 1, 1, 0, 0, 0)
    }
    .single()
    .expect("next month");
    PeriodClock {
        kind: PeriodKind::Monthly,
        id: format!("{}-{:02}", now.year(), now.month()),
        starts_at,
        resets_at: Some(next),
    }
}

pub fn seasonal(season_id: i32, starts_at: DateTime<Utc>, ends_at: DateTime<Utc>) -> PeriodClock {
    PeriodClock {
        kind: PeriodKind::Seasonal,
        id: format!("season:{season_id}"),
        starts_at,
        resets_at: Some(ends_at),
    }
}

pub fn lifetime() -> PeriodClock {
    PeriodClock {
        kind: PeriodKind::GettingStarted,
        id: "lifetime".into(),
        starts_at: DateTime::<Utc>::UNIX_EPOCH,
        resets_at: None,
    }
}

pub fn paid_ladder(
    season_id: i32,
    starts_at: DateTime<Utc>,
    ends_at: DateTime<Utc>,
) -> PeriodClock {
    let mut clock = seasonal(season_id, starts_at, ends_at);
    clock.kind = PeriodKind::PaidLadder;
    clock
}

/// Covering window so daily/weekly/monthly all sit in one match query.
pub fn covering_start(
    week: &PeriodClock,
    month: &PeriodClock,
    season: &PeriodClock,
) -> DateTime<Utc> {
    week.starts_at.min(month.starts_at).min(season.starts_at)
}

pub fn cache_ttl_secs(now: DateTime<Utc>) -> u64 {
    let day = daily(now);
    let until_midnight = day
        .resets_at
        .map(|end| (end - now).num_seconds().max(1) as u64)
        .unwrap_or(600);
    until_midnight.min(600)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daily_id_and_reset() {
        let now = Utc.with_ymd_and_hms(2026, 9, 1, 15, 30, 0).unwrap();
        let p = daily(now);
        assert_eq!(p.id, "2026-09-01");
        assert_eq!(p.resets_at.unwrap().date_naive().to_string(), "2026-09-02");
    }

    #[test]
    fn iso_week_straddles_year() {
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap();
        let p = weekly(now);
        assert!(p.id.starts_with("2026-W") || p.id.starts_with("2025-W"));
        assert_eq!(p.starts_at.weekday().num_days_from_monday(), 0);
    }

    #[test]
    fn monthly_september() {
        let now = Utc.with_ymd_and_hms(2026, 9, 15, 1, 0, 0).unwrap();
        let p = monthly(now);
        assert_eq!(p.id, "2026-09");
        assert_eq!(
            p.resets_at.unwrap(),
            Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap()
        );
    }
}
