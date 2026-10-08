use crate::calendar::{Calendar, Event};
use chrono::{DateTime, Duration, FixedOffset, Local};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const PENDING_LIMIT: usize = 64;
pub const PROFILE: &str = "native-preparation-v1";

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
    pub promotion: u64,
    pub verified: bool,
    pub state: State,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Flight {
    pub event_id: String,
    pub fingerprint: String,
    pub foreground: bool,
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
        }
    }
}

pub fn fingerprint(event: &Event) -> String {
    fingerprint_for(event, PROFILE)
}

fn default_analysis_revision() -> String {
    PROFILE.into()
}

fn fingerprint_for(event: &Event, analysis_revision: &str) -> String {
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
    event.end > now
        && event.start < now + Duration::days(7)
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
        if !self.flights.is_empty() {
            self.failure.get_or_insert_with(|| "Interrupted provider operation; inspect saved evidence and explicitly retry before resuming preparation".into());
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
        Ok(())
    }

    pub fn reconcile(&mut self, calendar: &Calendar, now: DateTime<FixedOffset>) -> Vec<u64> {
        let events = calendar
            .events
            .iter()
            .map(|event| (event.id.as_str(), event))
            .collect::<BTreeMap<_, _>>();
        for job in self.jobs.values_mut() {
            let event = events.get(job.event_id.as_str());
            if event.is_none_or(|event| !eligible(event, now)) {
                job.state = State::Cancelled;
                job.message =
                    "Meeting removed, cancelled, elapsed or outside preparation horizon".into();
            }
        }
        for event in calendar.events.iter().filter(|event| eligible(event, now)) {
            let key = fingerprint_for(event, &self.analysis_revision);
            let day = event.start.max(now).with_timezone(&Local).date_naive();
            let verified = calendar.coverage.contains(&day);
            let job = self.jobs.entry(event.id.clone()).or_insert_with(|| Job {
                event_id: event.id.clone(),
                fingerprint: key.clone(),
                start: event.start,
                foreground: false,
                promotion: 0,
                verified,
                state: State::Deferred,
                message: String::new(),
            });
            if job.fingerprint != key {
                let needs_retry = matches!(job.state, State::Failed | State::Interrupted);
                *job = Job {
                    event_id: event.id.clone(),
                    fingerprint: key,
                    start: event.start,
                    foreground: job.foreground,
                    promotion: job.promotion,
                    verified,
                    state: if needs_retry { job.state } else { State::Deferred },
                    message: if needs_retry {
                        "Meeting or AI configuration changed; inspect the previous outcome before explicit retry"
                    } else {
                        "Meeting or AI configuration changed"
                    }.into(),
                };
            }
            job.verified = verified;
            if job.state == State::Cancelled {
                job.state = State::Deferred;
            }
            if !verified && matches!(job.state, State::Queued | State::Deferred | State::Running) {
                job.state = State::Deferred;
                job.message = "Calendar coverage is unverified; no automatic analysis".into();
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
        let mut candidates = self
            .jobs
            .values()
            .filter(|job| job.verified && matches!(job.state, State::Deferred | State::Queued))
            .map(|job| {
                (
                    job.event_id.clone(),
                    job.foreground,
                    job.promotion,
                    job.start,
                )
            })
            .collect::<Vec<_>>();
        candidates.sort_by_key(|(id, foreground, promotion, start)| {
            (
                !*foreground,
                std::cmp::Reverse(*promotion),
                *start,
                id.clone(),
            )
        });
        for (index, (id, _, _, _)) in candidates.into_iter().enumerate() {
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

    fn candidate(&self) -> Option<&Job> {
        if self.flights.len() >= 2 {
            return None;
        }
        let background_busy = self.flights.values().any(|flight| !flight.foreground);
        self.jobs
            .values()
            .filter(|job| {
                job.state == State::Queued
                    && (job.foreground || !background_busy)
                    && !self
                        .flights
                        .values()
                        .any(|flight| flight.event_id == job.event_id)
            })
            .min_by_key(|job| {
                (
                    !job.foreground,
                    std::cmp::Reverse(job.promotion),
                    job.start,
                    &job.event_id,
                )
            })
    }

    pub fn can_dispatch(&self) -> bool {
        self.candidate().is_some()
    }

    pub fn dispatch(&mut self) -> Option<(u64, Flight)> {
        let job = self.candidate()?;
        let flight = Flight {
            event_id: job.event_id.clone(),
            fingerprint: job.fingerprint.clone(),
            foreground: job.foreground,
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
                    job.state = State::Completed;
                    job.message =
                        "Saved result validated; citations are not independently verified".into();
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
