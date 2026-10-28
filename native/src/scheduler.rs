use crate::calendar::{Calendar, Event};
use chrono::{DateTime, Duration, FixedOffset, Local};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewRequest {
    pub kinds: BTreeSet<crate::recommendations::Review>,
    pub until: DateTime<FixedOffset>,
}

impl ReviewRequest {
    pub fn includes(&self, kind: crate::recommendations::Kind) -> bool {
        self.kinds.iter().any(|review| review.includes(kind))
    }

    pub fn covers(&self, event: &Event, now: DateTime<FixedOffset>) -> bool {
        event.end > now
            && event.start < self.until
            && self.kinds.iter().any(|review| review.eligible(event))
    }
}

pub const PENDING_LIMIT: usize = 64;
pub const PROFILE: &str = "native-preparation-v1";
const LEGACY_SHUTDOWN_FAILURE: &str = "Preparation stopped after a failed operation: AI is stopping; the previous result was discarded; Cancelled before the question was sent. Inspect saved evidence and explicitly retry.";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum State {
    Deferred,
    Queued,
    Running,
    Completed,
    Failed,
    Interrupted,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    pub event_id: String,
    pub fingerprint: String,
    pub start: DateTime<FixedOffset>,
    pub foreground: bool,
    #[serde(default)]
    pub manual: bool,
    #[serde(default)]
    pub on_demand: bool,
    #[serde(default)]
    pub promotion: u64,
    pub verified: bool,
    pub state: State,
    pub message: String,
    #[serde(default)]
    pub retries: u8,
    #[serde(default)]
    pub retry_at: Option<DateTime<FixedOffset>>,
    #[serde(default)]
    pub prepared_at: Option<DateTime<FixedOffset>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<ReviewRequest>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Flight {
    pub event_id: String,
    pub fingerprint: String,
    pub foreground: bool,
    #[serde(default)]
    pub translation_only: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Queue {
    pub version: u32,
    #[serde(default = "default_analysis_revision")]
    pub analysis_revision: String,
    pub jobs: BTreeMap<String, Job>,
    pub flights: BTreeMap<u64, Flight>,
    pub sequence: u64,
    #[serde(default)]
    pub failure: Option<String>,
    #[serde(default)]
    pub budget_exhausted_day: Option<chrono::NaiveDate>,
}

impl Default for Queue {
    fn default() -> Self {
        Self {
            version: 1,
            analysis_revision: default_analysis_revision(),
            jobs: BTreeMap::new(),
            flights: BTreeMap::new(),
            sequence: 0,
            failure: None,
            budget_exhausted_day: None,
        }
    }
}

pub fn fingerprint(event: &Event) -> String {
    fingerprint_for(event, PROFILE)
}

fn default_analysis_revision() -> String {
    PROFILE.into()
}

pub(crate) fn fingerprint_for(event: &Event, analysis_revision: &str) -> String {
    // Exclude preparation status: publishing a briefing must not schedule itself again.
    let mut input = event.clone();
    input.briefing_status.clear();
    let json = serde_json::to_vec(&input).expect("Event contains only JSON-serializable values");
    use sha2::{Digest, Sha256};
    format!(
        "{:x}",
        Sha256::digest([analysis_revision.as_bytes(), &json].concat())
    )
}

pub fn eligible(event: &Event, now: DateTime<FixedOffset>) -> bool {
    event.accepted()
        && event.end > now
        && now
            .date_naive()
            .checked_add_months(chrono::Months::new(6))
            .is_some_and(|last| event.start.with_timezone(&Local).date_naive() <= last)
        && !matches!(
            event.status.to_ascii_lowercase().as_str(),
            "cancelled" | "canceled" | "deleted"
        )
        && event.extra.get("isCancelled").and_then(|v| v.as_bool()) != Some(true)
}

impl Queue {
    pub fn set_analysis_revision(&mut self, revision: &str) -> Result<bool, String> {
        if !valid_fingerprint(revision) {
            return Err("Invalid active AI configuration identity; preparation is stopped".into());
        }
        if self.analysis_revision == revision {
            return Ok(false);
        }
        self.analysis_revision = revision.into();
        for job in self.jobs.values_mut() {
            job.review = None;
        }
        Ok(true)
    }

    pub fn recover(&mut self) -> Result<(), String> {
        if self.version != 1 {
            return Err(format!(
                "Unsupported preparation ledger version {}",
                self.version
            ));
        }
        if self.sequence >= 1 << 63 {
            return Err("Preparation request identity limit reached".into());
        }
        if self.analysis_revision != PROFILE && !valid_fingerprint(&self.analysis_revision) {
            return Err("Preparation ledger contains an invalid AI configuration identity".into());
        }
        if self.jobs.iter().any(|(id, job)| {
            id.is_empty()
                || id != &job.event_id
                || !valid_fingerprint(&job.fingerprint)
                || job.promotion > self.sequence
        }) || self
            .jobs
            .values()
            .filter(|job| job.state == State::Queued)
            .count()
            > PENDING_LIMIT
        {
            return Err(
                "Preparation ledger contains invalid job identities or admission state".into(),
            );
        }
        let mut identities = std::collections::BTreeSet::new();
        if self.flights.len() > 2
            || self
                .flights
                .values()
                .filter(|flight| !flight.foreground)
                .count()
                > 1
            || self.flights.iter().any(|(id, flight)| {
                *id == 0
                    || *id > self.sequence
                    || !valid_fingerprint(&flight.fingerprint)
                    || !self.jobs.contains_key(&flight.event_id)
                    || !identities.insert(&flight.event_id)
            })
            || self.jobs.values().any(|job| {
                job.state == State::Running
                    && !self.flights.values().any(|flight| {
                        flight.event_id == job.event_id && flight.fingerprint == job.fingerprint
                    })
            })
        {
            return Err(
                "Preparation ledger contains invalid execution identities or concurrency".into(),
            );
        }
        // Repair only the known pre-send shutdown error. Unknown outcomes still require review.
        if self.failure.as_deref() == Some(LEGACY_SHUTDOWN_FAILURE) {
            eprintln!(
                "Tomlook: recovered legacy pre-send shutdown failure; saved Insights and explicit manual retry state retained"
            );
            self.failure = None;
        }
        // Research is read-only. An interrupted request must not halt unrelated meetings.
        if self.failure.as_deref() == Some("Interrupted provider operation; inspect saved evidence and explicitly retry before resuming preparation")
            || self.failure.as_deref() == Some("AI executor stopped before an outcome was recorded; inspect saved evidence before explicit retry")
        {
            self.failure = None;
        }
        for flight in self.flights.values() {
            if let Some(job) = self.jobs.get_mut(&flight.event_id) {
                job.state = State::Interrupted;
                job.message =
                    "Interrupted provider operation; inspect saved evidence before explicit retry"
                        .into();
            }
        }
        self.flights.clear();
        for job in self.jobs.values_mut().filter(|job| job.state == State::Interrupted && !job.manual) {
            if job.retries < 3 {
                job.retries += 1;
                job.state = State::Deferred;
                job.retry_at = Some(Local::now().fixed_offset() + Duration::seconds(60));
                job.message = "Read-only research interrupted; other meetings can continue".into();
            }
        }
        Ok(())
    }

    pub fn reconcile(&mut self, calendar: &Calendar, now: DateTime<FixedOffset>) -> Vec<u64> {
        self.update(calendar, now, true)
    }

    pub(crate) fn refresh(&mut self, calendar: &Calendar, now: DateTime<FixedOffset>) -> Vec<u64> {
        // The worker uses full reconciliation after calendar/configuration changes.
        self.update(calendar, now, false)
    }

    fn update(
        &mut self,
        calendar: &Calendar,
        now: DateTime<FixedOffset>,
        inputs_changed: bool,
    ) -> Vec<u64> {
        let events = calendar
            .events
            .iter()
            .map(|event| (event.id.as_str(), event))
            .collect::<BTreeMap<_, _>>();
        let flying = self.flights.values().map(|flight| flight.event_id.clone()).collect::<BTreeSet<_>>();
        self.jobs.retain(|id, job| job.manual || job.state == State::Completed
            || flying.contains(id) || events.get(id.as_str()).is_some_and(|event| eligible(event, now)));
        for job in self.jobs.values_mut() {
            let event = events.get(job.event_id.as_str());
            if event.is_none_or(|event| !job.manual && !eligible(event, now)) {
                if job.state != State::Completed {
                    job.state = State::Cancelled;
                }
                job.review = None;
                job.message =
                    "Meeting removed, cancelled, elapsed or outside preparation horizon".into();
            }
        }
        let relevant = calendar
            .events
            .iter()
            .filter(|event| {
                eligible(event, now) || self.jobs.get(&event.id).is_some_and(|job| job.manual)
            })
            .collect::<Vec<_>>();
        for event in relevant {
            let key = if !inputs_changed && let Some(job) = self.jobs.get(&event.id) {
                job.fingerprint.clone()
            } else {
                fingerprint_for(event, &self.analysis_revision)
            };
            let day = if self.jobs.get(&event.id).is_some_and(|job| job.manual) {
                event.start
            } else {
                event.start.max(now)
            }
            .with_timezone(&Local)
            .date_naive();
            let verified = calendar.coverage.contains(&day);
            let job = self.jobs.entry(event.id.clone()).or_insert_with(|| Job {
                event_id: event.id.clone(),
                fingerprint: key.clone(),
                start: event.start,
                foreground: false,
                manual: false,
                on_demand: event.unconfirmed(),
                promotion: 0,
                verified,
                state: State::Deferred,
                message: String::new(),
                retries: 0,
                retry_at: None,
                prepared_at: None,
                review: None,
            });
            if job.fingerprint != key {
                let needs_retry = matches!(job.state, State::Failed | State::Interrupted);
                *job = Job {
                    event_id: event.id.clone(),
                    fingerprint: key,
                    start: event.start,
                    foreground: job.foreground,
                    manual: job.manual,
                    on_demand: event.unconfirmed(),
                    promotion: job.promotion,
                    verified,
                    state: if needs_retry { job.state } else { State::Deferred },
                    message: if needs_retry {
                        "Meeting or AI configuration changed; inspect the previous outcome before explicit retry"
                    } else {
                        "Meeting or AI configuration changed"
                    }.into(),
                    retries: 0,
                    retry_at: None,
                    prepared_at: None,
                    review: job.review.clone(),
                };
            }
            if job
                .review
                .as_ref()
                .is_some_and(|review| !review.covers(event, now))
            {
                job.review = None;
                if job.state == State::Running && !job.manual {
                    job.state = State::Deferred;
                    job.message =
                        "Ad-hoc review window elapsed or the meeting is no longer eligible".into();
                }
            }
            if job.state == State::Completed
                && !job.manual
                && !event.unconfirmed()
                && event.start < now + Duration::days(14)
                && job
                    .prepared_at
                    .is_none_or(|prepared| now - prepared >= Duration::hours(24))
            {
                job.state = State::Deferred;
                job.retries = 0;
                job.message = "Refreshing upcoming meeting context".into();
            }
            job.verified = verified;
            job.on_demand = event.unconfirmed();
            if job.state == State::Cancelled && (job.manual || eligible(event, now)) {
                job.state = State::Deferred;
            }
            if !verified && matches!(job.state, State::Queued | State::Deferred | State::Running) {
                job.state = State::Deferred;
                job.message = "Calendar coverage is unverified; no automatic analysis".into();
            }
            if job.on_demand
                && !job.manual
                && !job.foreground
                && matches!(job.state, State::Queued | State::Deferred | State::Running)
            {
                job.state = State::Deferred;
                job.message = "Unconfirmed meeting: prepare explicitly when needed".into();
            }
        }
        self.admit();
        self.flights
            .iter()
            .filter(|(_, flight)| {
                self.jobs.get(&flight.event_id).is_none_or(|job| {
                    job.fingerprint != flight.fingerprint
                        || job.state != State::Running
                        || !job.verified
                })
            })
            .map(|(id, _)| *id)
            .collect()
    }

    fn admit(&mut self) {
        let now = Local::now().fixed_offset();
        let mut candidates = self
            .jobs
            .values()
            .filter(|job| {
                job.verified
                    && (!job.on_demand || job.manual || job.foreground)
                    && matches!(job.state, State::Deferred | State::Queued)
                    && job.retry_at.is_none_or(|retry| retry <= now)
            })
            .map(|job| {
                (
                    job.event_id.clone(),
                    job.foreground,
                    job.review.is_some(),
                    job.promotion,
                    job.start,
                )
            })
            .collect::<Vec<_>>();
        candidates.sort_by_key(|(id, foreground, review, promotion, start)| {
            (
                !*foreground,
                !*review,
                std::cmp::Reverse(*promotion),
                *start,
                id.clone(),
            )
        });
        for (index, (id, _, _, _, _)) in candidates.into_iter().enumerate() {
            let job = self.jobs.get_mut(&id).expect("candidate exists");
            job.state = if index < PENDING_LIMIT {
                State::Queued
            } else {
                State::Deferred
            };
            job.message = if index < PENDING_LIMIT {
                "Waiting for an execution slot"
            } else {
                "Deferred: bounded queue is full"
            }
            .into();
        }
    }

    pub fn promote(&mut self, id: &str) -> bool {
        let Some(job) = self.jobs.get_mut(id) else {
            return false;
        };
        if !matches!(job.state, State::Queued | State::Deferred) {
            return false;
        }
        if self.sequence == (1 << 63) - 1 {
            self.failure = Some(
                "Preparation request identity limit reached; no operation was submitted".into(),
            );
            return false;
        }
        self.sequence += 1;
        job.foreground = true;
        job.retry_at = None;
        job.promotion = self.sequence;
        self.admit();
        true
    }

    pub fn retry(&mut self, id: &str) -> bool {
        let Some(job) = self.jobs.get_mut(id) else {
            return false;
        };
        if !matches!(job.state, State::Failed | State::Interrupted) {
            return false;
        }
        job.state = State::Deferred;
        self.failure = None;
        self.promote(id)
    }

    pub fn request_now(&mut self, event: &Event, verified: bool) -> Result<(), String> {
        if !verified {
            return Err("Refresh this meeting's calendar before preparing it.".into());
        }
        if self.sequence >= (1 << 63) - 2 {
            return Err("Preparation identity limit reached; no operation was queued".into());
        }
        if self
            .flights
            .values()
            .any(|flight| flight.event_id == event.id)
        {
            return Err("This meeting is already being prepared.".into());
        }
        let fingerprint = fingerprint_for(event, &self.analysis_revision);
        self.jobs.insert(
            event.id.clone(),
            Job {
                event_id: event.id.clone(),
                fingerprint,
                start: event.start,
                foreground: true,
                manual: true,
                on_demand: event.unconfirmed(),
                promotion: 0,
                verified,
                state: State::Deferred,
                message: String::new(),
                retries: 0,
                retry_at: None,
                prepared_at: None,
                review: None,
            },
        );
        self.failure = None;
        if !self.promote(&event.id) {
            return Err("Preparation could not be queued.".into());
        }
        Ok(())
    }

    pub fn request_review(
        &mut self,
        calendar: &Calendar,
        now: DateTime<FixedOffset>,
        kind: crate::recommendations::Review,
    ) -> Result<usize, String> {
        if self.failure.is_some() {
            return Err(
                "Resolve the previous preparation failure before starting a calendar review."
                    .into(),
            );
        }
        let until = now + Duration::days(14);
        let first = now.with_timezone(&Local).date_naive();
        let last = (until - Duration::nanoseconds(1))
            .with_timezone(&Local)
            .date_naive();
        if (0..=(last - first).num_days())
            .any(|day| !calendar.coverage.contains(&(first + Duration::days(day))))
        {
            return Err(
                "Refresh the next 14 days of the calendar before starting this review.".into(),
            );
        }
        let mut next = self.clone();
        next.reconcile(calendar, now);
        let events = calendar
            .events
            .iter()
            .filter(|event| event.end > now && event.start < until && kind.eligible(event))
            .collect::<Vec<_>>();
        if events.iter().any(|event| {
            next.jobs
                .get(&event.id)
                .is_some_and(|job| matches!(job.state, State::Failed | State::Interrupted))
        }) {
            return Err("Some meetings have failed or interrupted research; inspect and retry them before this review.".into());
        }
        for event in &events {
            let job = next
                .jobs
                .get_mut(&event.id)
                .ok_or("Meeting could not enter the review queue.")?;
            if job.state == State::Completed {
                job.manual = false;
                job.promotion = 0;
            }
            if job.state != State::Running && !job.manual {
                job.foreground = false;
            }
            let review = job.review.get_or_insert_with(|| ReviewRequest {
                kinds: BTreeSet::new(),
                until,
            });
            review.kinds.insert(kind);
            review.until = review.until.max(until);
            if job.state != State::Running {
                job.state = State::Deferred;
                job.retry_at = None;
                job.retries = 0;
            }
        }
        next.admit();
        *self = next;
        Ok(events.len())
    }

    pub fn yield_for_manual(&mut self) -> Option<u64> {
        if self.flights.len() < 2 {
            return None;
        }
        let id = self
            .flights
            .iter()
            .filter(|(_, flight)| {
                self.jobs
                    .get(&flight.event_id)
                    .is_some_and(|job| !job.manual)
            })
            .min_by_key(|(id, flight)| (flight.foreground, **id))
            .map(|(id, _)| *id)?;
        self.defer(id, "Yielded to an explicit meeting preparation".into());
        Some(id)
    }

    fn candidate(&self, automatic: bool) -> Option<&Job> {
        if self.flights.len() >= 2 {
            return None;
        }
        let background_busy = self.flights.values().any(|flight| !flight.foreground);
        self.jobs
            .values()
            .filter(|job| {
                job.state == State::Queued
                    && (!job.on_demand || job.manual || job.foreground)
                    && (automatic || job.manual || job.review.is_some())
                    && (job.foreground || !background_busy)
                    && !self
                        .flights
                        .values()
                        .any(|flight| flight.event_id == job.event_id)
            })
            .min_by_key(|job| {
                (
                    !job.foreground,
                    job.review.is_none(),
                    std::cmp::Reverse(job.promotion),
                    job.start,
                    &job.event_id,
                )
            })
    }

    pub fn can_dispatch(&self) -> bool {
        self.candidate(true).is_some()
    }

    pub fn can_dispatch_for(&self, automatic: bool) -> bool {
        self.candidate(automatic).is_some()
    }

    pub fn dispatch(&mut self) -> Option<(u64, Flight)> {
        self.dispatch_for(true)
    }

    pub fn dispatch_for(&mut self, automatic: bool) -> Option<(u64, Flight)> {
        let job = self.candidate(automatic)?;
        let flight = Flight {
            event_id: job.event_id.clone(),
            fingerprint: job.fingerprint.clone(),
            foreground: job.foreground,
            translation_only: false,
        };
        if self.sequence == (1 << 63) - 1 {
            self.failure = Some(
                "Preparation request identity limit reached; no operation was submitted".into(),
            );
            return None;
        }
        self.sequence += 1;
        let id = self.sequence;
        self.flights.insert(id, flight.clone());
        self.jobs.get_mut(&flight.event_id)?.state = State::Running;
        self.admit();
        Some((id, flight))
    }

    pub fn current(&self, request: u64) -> Option<&Flight> {
        self.flights.get(&request).filter(|flight| {
            self.jobs.get(&flight.event_id).is_some_and(|job| {
                job.fingerprint == flight.fingerprint && job.state == State::Running
            })
        })
    }

    pub fn finish(&mut self, request: u64, result: Result<(), String>) {
        if let Some(flight) = self.flights.remove(&request)
            && let Some(job) = self.jobs.get_mut(&flight.event_id)
            && job.fingerprint == flight.fingerprint
            && job.state == State::Running
        {
            match result {
                Ok(()) => {
                    let review_after_translation = flight.translation_only && job.review.is_some();
                    job.state = if review_after_translation {
                        State::Deferred
                    } else {
                        State::Completed
                    };
                    job.prepared_at = Some(Local::now().fixed_offset());
                    job.message =
                        "Saved result validated; citations are not independently verified".into();
                    if !review_after_translation {
                        job.review = None;
                    }
                }
                Err(error) => {
                    job.state = State::Failed;
                    self.failure = Some(error.clone());
                    job.message = error;
                }
            }
        }
        self.admit();
    }

    pub fn background_failed(&mut self, request: u64, message: String) {
        let Some(flight) = self.current(request).cloned() else {
            return;
        };
        let previous_failure = self.failure.clone();
        self.finish(request, Err(message.clone()));
        self.failure = previous_failure;
        if let Some(job) = self.jobs.get_mut(&flight.event_id)
            && !job.manual
            && job.retries < 3
        {
            job.retries += 1;
            let delay = 30 * (1_i64 << job.retries);
            job.retry_at = Some(Local::now().fixed_offset() + Duration::seconds(delay));
            job.state = State::Deferred;
            job.message = format!(
                "Preparation failed: {message}. Retry {}/3 in {delay}s.",
                job.retries
            );
        }
        self.admit();
    }

    pub fn cancelled(&mut self, request: u64, message: String) {
        let Some(flight) = self.current(request).cloned() else {
            return;
        };
        self.flights.remove(&request);
        let job = self
            .jobs
            .get_mut(&flight.event_id)
            .expect("Current job exists");
        job.state = if job.manual || job.review.is_some() {
            State::Interrupted
        } else {
            State::Deferred
        };
        job.retry_at = if job.manual || job.review.is_some() {
            None
        } else {
            Some(Local::now().fixed_offset() + Duration::seconds(2))
        };
        job.message =
            format!("Insights cancelled: {message}. Previously saved Insights are unchanged.");
        self.admit();
    }

    pub fn interrupt(&mut self, message: &str) {
        for flight in self.flights.values() {
            if let Some(job) = self.jobs.get_mut(&flight.event_id)
                && job.state == State::Running
            {
                job.state = State::Interrupted;
                job.message = message.into();
            }
        }
        self.flights.clear();
        self.failure = Some(message.into());
        self.admit();
    }

    pub fn defer(&mut self, request: u64, message: String) {
        if let Some(flight) = self.flights.remove(&request)
            && let Some(job) = self.jobs.get_mut(&flight.event_id)
            && job.fingerprint == flight.fingerprint
            && job.state == State::Running
        {
            job.state = State::Deferred;
            job.message = message;
        }
        self.admit();
    }
}

fn valid_fingerprint(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn review_calendar(now: DateTime<FixedOffset>) -> Calendar {
        let mut events = crate::calendar::fixture(7);
        for (index, event) in events.iter_mut().enumerate() {
            event.title = format!("Review meeting {index}");
            event.status = "accepted".into();
            event.extra.clear();
            event.start = now + Duration::days(index as i64 + 1);
            event.end = event.start + Duration::hours(1);
            event.is_all_day = false;
        }
        events[1].start = now + Duration::days(14) - Duration::seconds(1);
        events[1].end = events[1].start + Duration::hours(1);
        events[2].start = now + Duration::days(14);
        events[2].end = events[2].start + Duration::hours(1);
        events[3].start = now - Duration::days(1);
        events[3].end = now - Duration::hours(1);
        events[4].status = "cancelled".into();
        events[5].status = "tentative".into();
        events[6].is_all_day = true;
        Calendar::build(
            events,
            (0..=14)
                .map(|day| now.with_timezone(&Local).date_naive() + Duration::days(day))
                .collect(),
            Vec::new(),
        )
        .unwrap()
    }

    #[test]
    fn ad_hoc_reviews_have_an_exact_horizon_and_one_background_slot_when_paused() {
        use crate::recommendations::Review;
        let now = Local::now().fixed_offset();
        let calendar = review_calendar(now);
        let mut queue = Queue::default();
        assert_eq!(
            queue
                .request_review(&calendar, now, Review::Travel)
                .unwrap(),
            2
        );
        let selected = queue
            .jobs
            .values()
            .filter(|job| job.review.is_some())
            .count();
        assert_eq!(selected, 2);
        let (id, flight) = queue.dispatch_for(false).unwrap();
        assert!(!flight.foreground);
        assert!(!queue.jobs[&flight.event_id].manual);
        assert!(queue.dispatch_for(false).is_none());
        assert_eq!(
            queue
                .request_review(&calendar, now, Review::Categories)
                .unwrap(),
            3
        );
        let scope = queue.jobs[&flight.event_id].review.as_ref().unwrap();
        assert_eq!(
            scope.kinds,
            BTreeSet::from([Review::Travel, Review::Categories])
        );
        assert_eq!(
            queue.flights.len(),
            1,
            "The running research is reused, not restarted"
        );
        assert_eq!(
            queue
                .jobs
                .values()
                .filter(|job| job.review.is_some())
                .count(),
            3
        );
        let persisted: Queue =
            serde_json::from_str(&serde_json::to_string(&queue).unwrap()).unwrap();
        assert_eq!(persisted, queue);
        queue.finish(id, Ok(()));
        assert!(queue.jobs[&flight.event_id].review.is_none());
        let (next, next_flight) = queue.dispatch_for(false).unwrap();
        assert!(!next_flight.foreground);
        queue.finish(next, Ok(()));
        let (last, _) = queue.dispatch_for(false).unwrap();
        queue.finish(last, Ok(()));
        assert!(queue.dispatch_for(false).is_none());
    }

    #[test]
    fn review_admission_is_atomic_for_missing_coverage_and_previous_failures() {
        use crate::recommendations::Review;
        let now = Local::now().fixed_offset();
        let mut calendar = review_calendar(now);
        calendar
            .coverage
            .remove(&(now.with_timezone(&Local).date_naive() + Duration::days(10)));
        let mut queue = Queue::default();
        let before = queue.clone();
        assert!(
            queue
                .request_review(&calendar, now, Review::Travel)
                .is_err()
        );
        assert_eq!(queue, before);
        calendar
            .coverage
            .insert(now.with_timezone(&Local).date_naive() + Duration::days(10));
        queue.reconcile(&calendar, now);
        let (id, _) = queue.dispatch().unwrap();
        queue.finish(id, Err("Unknown previous outcome".into()));
        let before = queue.clone();
        assert!(
            queue
                .request_review(&calendar, now, Review::Categories)
                .is_err()
        );
        assert_eq!(queue, before);
    }

    #[test]
    fn review_joining_a_translation_waits_for_fresh_research() {
        use crate::recommendations::Review;
        let now = Local::now().fixed_offset();
        let calendar = review_calendar(now);
        let mut queue = Queue::default();
        queue.reconcile(&calendar, now);
        let (translation, flight) = queue.dispatch().unwrap();
        queue
            .flights
            .get_mut(&translation)
            .unwrap()
            .translation_only = true;
        queue
            .request_review(&calendar, now, Review::Travel)
            .unwrap();
        queue.finish(translation, Ok(()));
        assert_ne!(queue.jobs[&flight.event_id].state, State::Completed);
        assert!(queue.jobs[&flight.event_id].review.is_some());
        let (research, fresh) = queue.dispatch_for(false).unwrap();
        assert_eq!(fresh.event_id, flight.event_id);
        assert!(!fresh.translation_only);
        queue.finish(research, Ok(()));
        assert!(queue.jobs[&flight.event_id].review.is_none());
    }

    #[test]
    fn review_requests_do_not_cross_account_or_analysis_configuration_bindings() {
        use crate::recommendations::Review;
        let now = Local::now().fixed_offset();
        let calendar = review_calendar(now);
        let mut queue = Queue::default();
        queue
            .request_review(&calendar, now, Review::Categories)
            .unwrap();
        assert!(queue.can_dispatch_for(false));
        queue.set_analysis_revision(&"b".repeat(64)).unwrap();
        queue.reconcile(&calendar, now);
        assert!(queue.jobs.values().all(|job| job.review.is_none()));
        assert!(!queue.can_dispatch_for(false));
    }

    #[test]
    fn stale_completed_promotions_cannot_take_the_interactive_slot_for_a_review() {
        use crate::recommendations::Review;
        let now = Local::now().fixed_offset();
        let calendar = review_calendar(now);
        let mut queue = Queue::default();
        queue.reconcile(&calendar, now);
        let event = calendar
            .events
            .iter()
            .find(|event| event.start == now + Duration::days(1))
            .unwrap();
        queue.promote(&event.id);
        let (id, flight) = queue.dispatch().unwrap();
        assert!(flight.foreground);
        queue.finish(id, Ok(()));
        queue.jobs.get_mut(&event.id).unwrap().prepared_at = Some(now - Duration::hours(25));
        queue
            .request_review(&calendar, now, Review::Travel)
            .unwrap();
        let (_, flight) = queue.dispatch_for(false).unwrap();
        assert!(!flight.foreground);
        assert!(queue.dispatch_for(false).is_none());
    }

    #[test]
    fn review_expiry_cancels_the_old_flight_and_completed_manual_jobs_review_in_background() {
        use crate::recommendations::{Kind, Review};
        let now = Local::now().fixed_offset();
        let calendar = review_calendar(now);
        let event = calendar
            .events
            .iter()
            .find(|event| event.start == now + Duration::days(1))
            .unwrap();
        let mut queue = Queue::default();
        queue.request_now(event, true).unwrap();
        let (id, _) = queue.dispatch_for(false).unwrap();
        queue.finish(id, Ok(()));
        queue
            .request_review(&calendar, now, Review::Travel)
            .unwrap();
        let job = &queue.jobs[&event.id];
        assert!(!job.foreground);
        assert!(!job.manual);
        let scope = job.review.as_ref().unwrap();
        assert!(scope.includes(Kind::Travel));
        assert!(scope.includes(Kind::TravelAfter));
        assert!(!scope.includes(Kind::Category));
        let (id, flight) = queue.dispatch_for(false).unwrap();
        assert!(
            queue
                .reconcile(&calendar, now + Duration::days(15))
                .contains(&id)
        );
        assert!(queue.current(id).is_none());
        assert!(queue.jobs[&flight.event_id].review.is_none());
    }

    #[test]
    fn unchanged_inputs_refresh_matches_full_reconciliation_across_time_and_results() {
        let now = Local::now().fixed_offset();
        let mut events = crate::calendar::fixture(8);
        for (index, event) in events.iter_mut().enumerate() {
            event.start = now + Duration::minutes(index as i64);
            event.end = event.start + Duration::days(3);
        }
        events[7].start = now + Duration::days(190);
        events[7].end = events[7].start + Duration::hours(1);
        let calendar = Calendar::build(
            events.clone(),
            events
                .iter()
                .map(|event| event.start.with_timezone(&Local).date_naive())
                .collect(),
            Vec::new(),
        )
        .unwrap();
        let mut initial = Queue::default();
        initial.reconcile(&calendar, now);
        let (completed, _) = initial.dispatch().unwrap();
        initial.finish(completed, Ok(()));
        let (running, _) = initial.dispatch().unwrap();
        for elapsed in [
            Duration::zero(),
            Duration::hours(25),
            Duration::days(4),
            Duration::days(200),
        ] {
            let mut expected = initial.clone();
            let mut actual = initial.clone();
            let expected_cancelled = expected.reconcile(&calendar, now + elapsed);
            assert_eq!(actual.refresh(&calendar, now + elapsed), expected_cancelled);
            assert_eq!(actual, expected);
            if elapsed >= Duration::days(4) {
                assert!(expected_cancelled.contains(&running));
                assert!(actual.current(running).is_none());
            }
        }
    }

    #[test]
    fn changed_inputs_and_configuration_still_rebind_running_jobs() {
        let now = Local::now().fixed_offset();
        let mut event = crate::calendar::fixture(1).remove(0);
        event.start = now + Duration::minutes(10);
        event.end = event.start + Duration::hours(1);
        let mut calendar = Calendar::build(
            vec![event],
            [now.with_timezone(&Local).date_naive()]
                .into_iter()
                .collect(),
            Vec::new(),
        )
        .unwrap();
        let mut queue = Queue::default();
        queue.reconcile(&calendar, now);
        let (request, original) = queue.dispatch().unwrap();
        calendar.events[0].title = "Changed fixture".into();
        assert_eq!(queue.reconcile(&calendar, now), vec![request]);
        assert!(queue.current(request).is_none());
        queue.finish(request, Ok(()));
        let (next, changed) = queue.dispatch().unwrap();
        assert_ne!(changed.fingerprint, original.fingerprint);
        queue.set_analysis_revision(&"b".repeat(64)).unwrap();
        assert_eq!(queue.reconcile(&calendar, now), vec![next]);
        assert!(queue.current(next).is_none());
        assert_ne!(
            queue.jobs[&changed.event_id].fingerprint,
            changed.fingerprint
        );
    }
}
