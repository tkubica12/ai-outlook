use crate::workiq::connection::Range;
use chrono::{DateTime, Duration, Months, NaiveDate, Utc};
use std::collections::BTreeMap;

pub type Freshness = BTreeMap<NaiveDate, DateTime<Utc>>;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    pub fresh: usize,
    pub total: usize,
    pub through: Option<NaiveDate>,
    pub updated: Option<DateTime<Utc>>,
    pub fetching: bool,
    pub error: Option<String>,
}

pub struct Window {
    pub first: NaiveDate,
    pub today: NaiveDate,
    pub last: NaiveDate,
}

impl Window {
    pub fn new(today: NaiveDate) -> Result<Self, String> {
        Ok(Self {
            first: today
                .checked_sub_months(Months::new(1))
                .ok_or("Calendar synchronization start is outside supported dates")?,
            today,
            last: today
                .checked_add_months(Months::new(6))
                .ok_or("Calendar synchronization end is outside supported dates")?,
        })
    }

    fn fresh(&self, day: NaiveDate, stamps: &Freshness, now: DateTime<Utc>) -> bool {
        let ttl = if (day - self.today).num_days().abs() <= 14 {
            Duration::minutes(15)
        } else {
            Duration::hours(6)
        };
        stamps
            .get(&day)
            .is_some_and(|stamp| *stamp <= now && now - *stamp < ttl)
    }

    pub fn next(&self, stamps: &Freshness, now: DateTime<Utc>) -> Option<Range> {
        let future = (0..=(self.last - self.today).num_days())
            .map(|offset| self.today + Duration::days(offset));
        let past = (0..(self.today - self.first).num_days())
            .map(|offset| self.first + Duration::days(offset));
        let first = future
            .chain(past)
            .find(|day| !self.fresh(*day, stamps, now))?;
        let limit = if first < self.today {
            self.today - Duration::days(1)
        } else {
            self.last
        };
        let mut last = first;
        for offset in 1..42 {
            let day = first + Duration::days(offset);
            if day > limit || self.fresh(day, stamps, now) {
                break;
            }
            last = day;
        }
        Some(Range { first, last })
    }

    pub fn progress(&self, stamps: &Freshness, now: DateTime<Utc>) -> Progress {
        let total = (self.last - self.first).num_days() as usize + 1;
        Progress {
            fresh: (0..total)
                .filter(|offset| {
                    self.fresh(self.first + Duration::days(*offset as i64), stamps, now)
                })
                .count(),
            total,
            through: Some(self.last),
            updated: stamps.values().max().copied(),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backfill_is_bounded_future_first_and_resumes_from_persisted_days() {
        let now = DateTime::parse_from_rfc3339("2026-10-09T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let window = Window::new(now.date_naive()).unwrap();
        assert_eq!(window.first.to_string(), "2026-09-09");
        assert_eq!(window.last.to_string(), "2027-04-09");
        let mut stamps = Freshness::new();
        let mut requests = 0;
        while let Some(range) = window.next(&stamps, now) {
            if requests == 0 {
                assert_eq!(range.first, window.today);
            }
            assert!((range.last - range.first).num_days() < 42);
            for offset in 0..=(range.last - range.first).num_days() {
                stamps.insert(range.first + Duration::days(offset), now);
            }
            requests += 1;
            assert!(requests <= 7);
        }
        let progress = window.progress(&stamps, now);
        assert_eq!(progress.fresh, progress.total);
        let soon = window.next(&stamps, now + Duration::minutes(16)).unwrap();
        assert_eq!(soon.first, window.today);
        assert_eq!(soon.last, window.today + Duration::days(14));
        assert!(window.next(&stamps, now - Duration::minutes(1)).is_some());
    }

    #[test]
    fn missing_days_and_month_boundaries_are_not_skipped() {
        let today = NaiveDate::from_ymd_opt(2026, 8, 31).unwrap();
        let window = Window::new(today).unwrap();
        assert_eq!(window.last.to_string(), "2027-02-28");
        let now = Utc::now();
        let stamps = Freshness::from([(today + Duration::days(1), now)]);
        assert_eq!(window.next(&stamps, now).unwrap().last, today);
        assert!(Window::new(NaiveDate::MAX).is_err());
    }
}
