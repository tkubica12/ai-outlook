use chrono::{DateTime, Datelike, Duration, FixedOffset, Local, NaiveDate, Timelike};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Event {
    pub id: String,
    pub title: String,
    pub start: DateTime<FixedOffset>,
    pub end: DateTime<FixedOffset>,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub attendees: Vec<String>,
    #[serde(default)]
    pub organizer: String,
    #[serde(default)]
    pub location: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub briefing_status: String,
    #[serde(default)]
    pub is_all_day: bool,
    #[serde(default)]
    pub source_url: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

impl Event {
    pub fn validate(&self) -> Result<(), String> {
        if self.id.is_empty() || self.title.trim().is_empty() || self.end <= self.start {
            return Err("Meeting requires an identity, title and increasing zoned times".into());
        }
        if self.end - self.start > Duration::days(366) {
            return Err("Meeting duration exceeds one year".into());
        }
        Ok(())
    }

    pub fn fingerprint(&self) -> String {
        let mut digest = Sha256::new();
        for field in [
            self.id.as_str(),
            self.title.as_str(),
            &self.start.to_rfc3339(),
            &self.end.to_rfc3339(),
            self.organizer.as_str(),
            self.location.as_str(),
        ] {
            digest.update(field.len().to_le_bytes());
            digest.update(field.as_bytes());
        }
        for attendee in &self.attendees {
            digest.update(attendee.len().to_le_bytes());
            digest.update(attendee.as_bytes());
        }
        digest.update(b"tomlook-readonly-v1");
        format!("{:x}", digest.finalize())
    }

    pub fn all_day_like(&self) -> bool {
        self.is_all_day
            || self.end - self.start >= Duration::hours(12)
            || self.start.with_timezone(&Local).date_naive()
                != (self.end - Duration::milliseconds(1))
                    .with_timezone(&Local)
                    .date_naive()
    }
}

pub fn safe_url(value: &str) -> bool {
    if value.chars().any(char::is_control) {
        return false;
    }
    url::Url::parse(value).is_ok_and(|url| {
        matches!(url.scheme(), "http" | "https")
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
    })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum View {
    Day,
    #[default]
    WorkWeek,
    Week,
    Month,
}

impl View {
    pub const ALL: [(Self, &'static str); 4] = [
        (Self::Day, "Day"),
        (Self::WorkWeek, "Work week"),
        (Self::Week, "Week"),
        (Self::Month, "Month"),
    ];

    pub fn days(self, anchor: NaiveDate) -> Vec<NaiveDate> {
        let first = match self {
            Self::Day => anchor,
            Self::Month => monday(anchor.with_day(1).expect("valid day one")),
            _ => monday(anchor),
        };
        let count = match self {
            Self::Day => 1,
            Self::WorkWeek => 5,
            Self::Week => 7,
            Self::Month => 42,
        };
        (0..count).map(|i| first + Duration::days(i)).collect()
    }

    pub fn shift(self, anchor: NaiveDate, direction: i32) -> NaiveDate {
        if self != Self::Month {
            return anchor
                + Duration::days(i64::from(direction) * if self == Self::Day { 1 } else { 7 });
        }
        let total = anchor.year() * 12 + anchor.month0() as i32 + direction;
        let year = total.div_euclid(12);
        let month = total.rem_euclid(12) as u32 + 1;
        (1..=anchor.day())
            .rev()
            .find_map(|day| NaiveDate::from_ymd_opt(year, month, day))
            .expect("supported calendar year")
    }
}

pub fn monday(day: NaiveDate) -> NaiveDate {
    day - Duration::days(i64::from(day.weekday().num_days_from_monday()))
}

#[derive(Clone, Debug)]
pub struct Slot {
    pub event: usize,
    pub lane: usize,
    pub lanes: usize,
    pub start_minute: f32,
    pub end_minute: f32,
}

#[derive(Default, Clone, Debug)]
pub struct Day {
    pub events: Vec<usize>,
    pub timed: Vec<Slot>,
    pub all_day: Vec<usize>,
}

#[derive(Default, Clone)]
pub struct Calendar {
    pub events: Vec<Event>,
    pub days: BTreeMap<NaiveDate, Day>,
    pub coverage: BTreeSet<NaiveDate>,
    pub search: Vec<String>,
    pub categories: Vec<String>,
    pub warnings: Vec<String>,
}

impl Calendar {
    pub fn build(
        mut events: Vec<Event>,
        coverage: BTreeSet<NaiveDate>,
        warnings: Vec<String>,
    ) -> Result<Self, String> {
        let mut ids = BTreeSet::new();
        for event in &events {
            event.validate()?;
            if !ids.insert(event.id.clone()) {
                return Err("Calendar contains duplicate event identities".into());
            }
        }
        events.sort_by(|a, b| a.start.cmp(&b.start).then(a.id.cmp(&b.id)));
        let mut result = Self {
            events,
            coverage,
            warnings,
            ..Default::default()
        };
        result.categories = result
            .events
            .iter()
            .map(|event| event.category.clone())
            .filter(|category| !category.is_empty())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        for (index, event) in result.events.iter().enumerate() {
            result.search.push(
                format!(
                    "{} {} {} {} {}",
                    event.title,
                    event.organizer,
                    event.location,
                    event.category,
                    event.attendees.join(" ")
                )
                .to_lowercase(),
            );
            let start = event.start.with_timezone(&Local);
            let end = event.end.with_timezone(&Local);
            let last = (end - Duration::milliseconds(1)).date_naive();
            let mut date = start.date_naive();
            while date <= last {
                let day = result.days.entry(date).or_default();
                day.events.push(index);
                if event.all_day_like() {
                    day.all_day.push(index);
                } else {
                    day.timed.push(Slot {
                        event: index,
                        lane: 0,
                        lanes: 1,
                        start_minute: (start.hour() * 60 + start.minute()) as f32,
                        end_minute: if end.date_naive() != date {
                            1440.0
                        } else {
                            (end.hour() * 60 + end.minute()) as f32
                        },
                    });
                }
                date += Duration::days(1);
            }
        }
        for day in result.days.values_mut() {
            let mut group_start = 0;
            let mut group_end = -1.0_f32;
            let mut lane_ends = Vec::<f32>::new();
            for position in 0..day.timed.len() {
                let slot = &day.timed[position];
                if slot.start_minute >= group_end && position > group_start {
                    let lanes = lane_ends.len();
                    for prior in &mut day.timed[group_start..position] {
                        prior.lanes = lanes;
                    }
                    lane_ends.clear();
                    group_start = position;
                }
                let slot = &mut day.timed[position];
                let lane = lane_ends
                    .iter()
                    .position(|end| *end <= slot.start_minute)
                    .unwrap_or(lane_ends.len());
                if lane == lane_ends.len() {
                    lane_ends.push(slot.end_minute);
                } else {
                    lane_ends[lane] = slot.end_minute;
                }
                slot.lane = lane;
                group_end = if position == group_start {
                    slot.end_minute
                } else {
                    group_end.max(slot.end_minute)
                };
            }
            let lanes = lane_ends.len();
            for slot in &mut day.timed[group_start..] {
                slot.lanes = lanes;
            }
        }
        Ok(result)
    }

    pub fn matches(&self, index: usize, query: &str, category: &str) -> bool {
        self.search[index].contains(query)
            && (category.is_empty() || self.events[index].category == category)
    }
}

pub const FIXTURE_VERSION: &str = "tomlook-frozen-fixture-v1";

pub fn fixture_anchor() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, 5).expect("valid fixture anchor")
}

/// Deterministic benchmark corpus independent of the current date and machine timezone.
/// Offsets follow Central European rules; 25 October 2026 is the DST-end boundary.
pub fn fixture(count: usize) -> Vec<Event> {
    let anchor = fixture_anchor();
    let dst_end = NaiveDate::from_ymd_opt(2026, 10, 25).expect("valid DST boundary");
    let zoned = |date: NaiveDate, hour: u32, minute: u32| {
        let offset = if date < dst_end || (date == dst_end && hour < 2) {
            2
        } else {
            1
        };
        date.and_hms_opt(hour, minute, 0)
            .expect("valid fixture time")
            .and_local_timezone(FixedOffset::east_opt(offset * 3600).expect("valid offset"))
            .single()
            .expect("fixed offsets are unambiguous")
    };
    let subjects = [
        "Product direction",
        "Customer architecture review",
        "Design focus",
        "Delivery checkpoint",
        "Engineering workshop",
        "Weekly planning",
    ];
    let czech = "Česká porada – připravujeme další krok: přehled závazků, rizik, \
                 rozpočtu a odpovědností pro žluťoučký kůň úpěl ďábelské ódy";
    (0..count)
        .map(|i| {
            let kind = i % 50;
            let date = anchor + Duration::days((i % 42) as i64);
            let hour = 8 + ((i / 42) % 9) as u32;
            let minute = if i % 3 == 0 { 15 } else { 0 };
            let (start, end, all_day, title) = match kind {
                0 => {
                    let start = zoned(date, 0, 0);
                    (
                        start,
                        zoned(date + Duration::days(1), 0, 0),
                        true,
                        "All-day planning".to_string(),
                    )
                }
                1 => {
                    let first = anchor + Duration::days((i % 40) as i64);
                    (
                        zoned(first, 9, 0),
                        zoned(first + Duration::days(2), 17, 0),
                        false,
                        "Multi-day offsite".to_string(),
                    )
                }
                2 => (
                    zoned(dst_end, 1, 30),
                    zoned(dst_end, 3, 30),
                    false,
                    "DST boundary checkpoint".to_string(),
                ),
                _ => {
                    let start = zoned(date, hour, minute);
                    let title = if i % 10 == 7 {
                        format!("{czech} #{i}")
                    } else {
                        subjects[i % subjects.len()].to_string()
                    };
                    (
                        start,
                        start + Duration::minutes(if i % 5 == 0 { 90 } else { 45 }),
                        false,
                        title,
                    )
                }
            };
            Event {
                id: format!("fixture-{i:05}"),
                title,
                start,
                end,
                category: ["Customer", "Internal", "Focus"][i % 3].into(),
                attendees: vec!["Alex (synthetic)".into(), "Jamie (synthetic)".into()],
                organizer: "Morgan (synthetic)".into(),
                location: if i % 2 == 0 { "Online" } else { "Room 2" }.into(),
                status: "accepted".into(),
                briefing_status: "not_ready".into(),
                is_all_day: all_day,
                source_url: None,
                extra: BTreeMap::new(),
            }
        })
        .collect()
}

pub fn fixture_coverage() -> BTreeSet<NaiveDate> {
    (0..42)
        .map(|i| fixture_anchor() + Duration::days(i))
        .collect()
}

pub fn demo(now: DateTime<Local>, count: usize) -> Vec<Event> {
    let subjects = [
        "Product direction",
        "Customer architecture review",
        "Design focus",
        "Delivery checkpoint",
        "Engineering workshop",
        "Weekly planning",
        "Czech team - pripravujeme dalsi krok",
    ];
    (0..count)
        .map(|i| {
            let date = monday(now.date_naive()) + Duration::days((i % 42) as i64);
            let hour = 8 + ((i / 42) % 9) as u32;
            let start = date
                .and_hms_opt(hour, if i % 3 == 0 { 15 } else { 0 }, 0)
                .expect("valid synthetic time")
                .and_local_timezone(Local)
                .earliest()
                .expect("synthetic daytime exists")
                .fixed_offset();
            Event {
                id: format!("synthetic-{i}"),
                title: subjects[i % subjects.len()].into(),
                start,
                end: start + Duration::minutes(if i % 5 == 0 { 90 } else { 45 }),
                category: ["Customer", "Internal", "Focus"][i % 3].into(),
                attendees: vec!["Alex (synthetic)".into(), "Jamie (synthetic)".into()],
                organizer: "Morgan (synthetic)".into(),
                location: if i % 2 == 0 { "Online" } else { "Room 2" }.into(),
                status: "accepted".into(),
                briefing_status: "not_ready".into(),
                is_all_day: false,
                source_url: None,
                extra: BTreeMap::new(),
            }
        })
        .collect()
}
