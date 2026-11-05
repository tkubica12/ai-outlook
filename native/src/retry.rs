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
    pub fn deadline(&self) -> Option<Instant> {
        self.deadline
    }
    pub fn started(&mut self) {
        self.deadline = None;
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

/// Automatic browser sign-in after a saved sign-in is rejected, at most once per interval.
#[derive(Default)]
pub struct SignIn {
    last: Option<Instant>,
    deadline: Option<Instant>,
}

impl SignIn {
    pub const INTERVAL: Duration = Duration::from_secs(3600);

    pub fn deadline(&self) -> Option<Instant> {
        self.deadline
    }
    pub fn schedule(&mut self, now: Instant) -> Duration {
        let at = self
            .last
            .map_or(now, |last| (last + Self::INTERVAL).max(now));
        self.deadline = Some(at);
        at - now
    }
    /// Consumes a due deadline and starts the rate-limit interval.
    pub fn take_due(&mut self, now: Instant) -> bool {
        if self.deadline.is_some_and(|deadline| now >= deadline) {
            self.deadline = None;
            self.last = Some(now);
            return true;
        }
        false
    }
    pub fn cancel(&mut self) {
        self.deadline = None;
    }
}

/// Errors only an interactive browser sign-in can resolve.
pub fn needs_sign_in(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    !transient(&error)
        && [
            "authorization required",
            "refresh token rejected",
            "token exchange rejected",
            "needs sign-in",
            "sign in again",
            "http 401",
            "browser authorization returned",
        ]
        .iter()
        .any(|reason| error.contains(reason))
}

pub fn transient(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
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
        "authorization required",
        "authorization failed",
        "refresh token rejected",
        "token exchange rejected",
        "credential rejected",
        "identity is not accepted",
        "identity does not match",
        "no compatible mcp protocol",
        "unsupported schema",
        "unsupported response type",
        "invalid mcp",
        "invalid tool arguments",
        "missing session identifier",
        "unexpected json",
        "unsupported content type",
        "os error 2",
        "os error 193",
    ]
    .iter()
    .any(|reason| error.contains(&reason.to_ascii_lowercase()))
    {
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
        "cannot reach the provider",
        "transport failed",
        "handshake failed",
        "connection/lifecycle failure",
        "OAuth HTTP request failed",
        "token refresh failed",
        "tool discovery failed",
        "connection failed",
        "connection closed",
        "mcp session expired; reconnect",
        "mcp transport closed",
        "transport/protocol failed",
        "transport/protocol failure",
        "sdk ping failed",
        "sdk authentication check failed",
        "did not complete its handshake",
    ]
    .iter()
    .any(|reason| error.contains(&reason.to_ascii_lowercase()))
}

pub fn failure_message(error: String, delay: Option<Duration>) -> String {
    match delay {
        Some(delay) => format!(
            "{error}\nAutomatic reconnect in about {} seconds.",
            delay.as_secs()
        ),
        None => error,
    }
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

    #[test]
    fn provider_transport_errors_recover_but_identity_and_protocol_errors_do_not() {
        for error in [
            "WebIQ initialization failed: cannot reach the provider",
            "GitHub initialization failed: MCP session expired; reconnect",
            "WebIQ tool discovery failed: MCP transport closed before the response",
            "Authenticated hosted WorkIQ initialization failed: transport/protocol failure; provider content withheld",
            "GitHub initialization failed: transport/protocol failed",
            "SDK ping failed: connection closed",
            "SDK authentication check failed: transport closed",
            "The isolated Copilot runtime did not complete its handshake in 30 seconds",
            "Hosted WorkIQ discovery failed without an OAuth challenge: transport/protocol failure; no credentials or tools were used",
            "Hosted WorkIQ discovery failed without an OAuth challenge: HTTP 503; no credentials or tools were used",
            "WorkIQ resource metadata could not reach the provider",
            "Hosted WorkIQ token refresh failed; credential details withheld",
        ] {
            assert!(transient(error), "{error}");
            assert!(!needs_sign_in(error), "{error}");
        }
        for error in [
            "Hosted WorkIQ authorization required; credential details withheld",
            "Hosted WorkIQ refresh token rejected (AADSTS700082); credential details withheld",
            "WebIQ initialization failed: HTTP 401: credential rejected",
            "GitHub tool discovery failed: invalid tool arguments (MCP code -32602)",
            "Connector read tool has an unsupported schema",
            "Authenticated hosted WorkIQ initialization failed: transport missing session identifier",
            "Isolated SDK handshake failed: system cannot find the file (os error 2)",
            "SDK authentication check failed: HTTP 403",
            "Isolated Copilot identity is not accepted: not signed in",
        ] {
            assert!(!transient(error), "{error}");
        }
    }

    #[test]
    fn rejected_sign_in_opens_the_browser_at_most_once_per_hour() {
        for error in [
            "Hosted WorkIQ authorization required; credential details withheld",
            "Hosted WorkIQ refresh token rejected (AADSTS700082); credential details withheld",
            "WorkIQ needs sign-in. Click the status bar to connect.",
            "Authenticated hosted WorkIQ initialization failed: HTTP 401; provider content withheld",
            "WorkIQ browser authorization returned interaction_required",
        ] {
            assert!(needs_sign_in(error), "{error}");
        }
        for error in [
            "WorkIQ signed-in account does not match the explicitly selected work account",
            "Could not bind the registered loopback callback on 127.0.0.1:8080; no browser was opened",
            "WorkIQ browser sign-in exceeded its 10-minute deadline",
        ] {
            assert!(!needs_sign_in(error), "{error}");
        }
        let now = Instant::now();
        let mut sign_in = SignIn::default();
        assert!(!sign_in.take_due(now));
        assert_eq!(sign_in.schedule(now), Duration::ZERO);
        assert!(sign_in.take_due(now));
        assert!(!sign_in.take_due(now));
        let later = now + Duration::from_secs(60);
        assert_eq!(sign_in.schedule(later), SignIn::INTERVAL - Duration::from_secs(60));
        assert!(!sign_in.take_due(later));
        sign_in.cancel();
        assert!(sign_in.deadline().is_none());
        sign_in.schedule(later);
        assert!(sign_in.take_due(now + SignIn::INTERVAL));
    }

    #[test]
    fn starting_consumes_only_the_deadline_and_success_or_cancel_resets_backoff() {
        let now = Instant::now();
        let mut retry = Retry::default();
        assert_eq!(
            retry.schedule(now, "WorkIQ transport failed"),
            Some(Duration::from_secs(30))
        );
        assert_eq!(retry.deadline(), Some(now + Duration::from_secs(30)));
        retry.started();
        assert!(!retry.pending());
        assert!(!retry.due(now + Duration::from_secs(3600)));
        assert_eq!(
            retry.schedule(now, "MCP transport closed"),
            Some(Duration::from_secs(60))
        );
        retry.clear();
        assert_eq!(
            retry.schedule(now, "HTTP 503"),
            Some(Duration::from_secs(30))
        );
        retry.clear();
        assert!(retry.deadline().is_none());
        assert_eq!(
            failure_message("HTTP 503".into(), Some(Duration::from_secs(30))),
            "HTTP 503\nAutomatic reconnect in about 30 seconds."
        );
        assert_eq!(failure_message("HTTP 401".into(), None), "HTTP 401");
    }
}
