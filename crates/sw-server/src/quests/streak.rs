//! Streak walks over distinct UTC play dates. No extra table.

use chrono::NaiveDate;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Streak {
    pub current: i32,
    pub longest: i32,
    pub last_active_date: Option<NaiveDate>,
}

pub fn from_sorted_unique_dates(dates: &[NaiveDate]) -> Streak {
    if dates.is_empty() {
        return Streak {
            current: 0,
            longest: 0,
            last_active_date: None,
        };
    }

    let mut longest = 1;
    let mut run = 1;
    for window in dates.windows(2) {
        let gap = (window[1] - window[0]).num_days();
        if gap == 1 {
            run += 1;
            longest = longest.max(run);
        } else if gap > 1 {
            run = 1;
        }
    }

    let last = *dates.last().expect("non-empty");
    let mut current = 1;
    for i in (1..dates.len()).rev() {
        let gap = (dates[i] - dates[i - 1]).num_days();
        if gap == 1 {
            current += 1;
        } else if gap > 1 {
            break;
        }
    }

    Streak {
        current,
        longest,
        last_active_date: Some(last),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn empty() {
        let s = from_sorted_unique_dates(&[]);
        assert_eq!(s.current, 0);
        assert_eq!(s.longest, 0);
        assert!(s.last_active_date.is_none());
    }

    #[test]
    fn consecutive_and_gap() {
        let dates = [d(2026, 9, 1), d(2026, 9, 2), d(2026, 9, 4)];
        let s = from_sorted_unique_dates(&dates);
        assert_eq!(s.current, 1);
        assert_eq!(s.longest, 2);
        assert_eq!(s.last_active_date, Some(d(2026, 9, 4)));
    }

    #[test]
    fn current_is_tail_run() {
        let dates = [d(2026, 9, 1), d(2026, 9, 3), d(2026, 9, 4), d(2026, 9, 5)];
        let s = from_sorted_unique_dates(&dates);
        assert_eq!(s.current, 3);
        assert_eq!(s.longest, 3);
    }

    #[test]
    fn same_day_deduped_upstream() {
        let dates = [d(2026, 9, 1)];
        let s = from_sorted_unique_dates(&dates);
        assert_eq!(s.current, 1);
        assert_eq!(s.longest, 1);
    }
}
