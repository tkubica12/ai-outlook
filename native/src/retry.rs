use std::time::{Duration, Instant};

#[derive(Default)]
pub struct Retry {
    attempts: u32,
    deadline: Option<Instant>,
}

impl Retry {
    pub fn pending(&self) -> bool {
        self.deadline.is_some()
    }
    pub fn due(&self, now: Instant) -> bool {
        self.deadline.is_some_and(|deadline| now >= deadline)
    }
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    pub fn schedule(&mut self, now: Instant, error: &str) -> Option<Duration> {
        if !transient(error) {
            self.clear();
            return None;
        }
        let delay = Duration::from_secs((30_u64 << self.attempts.min(5)).min(900));
        self.attempts = self.attempts.saturating_add(1);
        self.deadline = Some(now + delay);
        Some(delay)
    }
}

pub fn transient(error: &str) -> bool {
    if [
        "HTTP 400",
        "HTTP 401",
        "HTTP 403",
        "HTTP 404",
        "invalid_grant",
        "expired; sign in",
        "expired; sign in again",
        "no refresh token",
        "cancelled",
        "during shutdown",
        "permission",
        "not signed in",
        "no saved",
        "untrusted",
        "certificate",
    ]
    .iter()
    .any(|reason| {
        error
            .to_ascii_lowercase()
            .contains(&reason.to_ascii_lowercase())
    }) {
        return false;
    }
    [
        "HTTP 408",
        "HTTP 429",
        "HTTP 500",
        "HTTP 502",
        "HTTP 503",
        "HTTP 504",
        "timed out",
        "timeout",
        "deadline",
        "could not reach",
        "transport failed",
        "handshake failed",
        "connection/lifecycle failure",
        "OAuth HTTP request failed",
        "tool discovery failed",
        "connection failed",
        "connection closed",
    ]
    .iter()
    .any(|reason| {
        error
            .to_ascii_lowercase()
            .contains(&reason.to_ascii_lowercase())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn backoff_continues_indefinitely_but_never_retries_auth_or_cancel() {
        let now = Instant::now();
        let mut retry = Retry::default();
        for attempt in 0..30 {
            let delay = retry
                .schedule(now, "Connector read failed: HTTP 429")
                .unwrap();
            assert_eq!(delay.as_secs(), (30_u64 << attempt.min(5)).min(900));
            assert!(!retry.due(now));
            assert!(retry.due(now + delay));
        }
        for error in [
            "HTTP 401",
            "HTTP 403",
            "invalid_grant",
            "Connector sign-in cancelled",
            "Saved sign-in has no refresh token",
            "unsupported schema",
            "certificate untrusted",
        ] {
            assert!(retry.schedule(now, error).is_none());
            assert!(!retry.due(now + Duration::from_secs(86400)));
        }
    }
}
