//! Per-error-class retry decisions (pattern #9: FDM's
//! `DownloadErrorAllowedToAutoRetry` map, rebuilt for uploads).
//! No hardcoded retry logic scattered through adapters — policy lives here.

use std::time::Duration;

/// Classification every adapter must attach to its errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// 5xx, connection reset, timeout → retry with backoff.
    Transient,
    /// 429 / quota → retry honoring `Retry-After`.
    Throttled,
    /// Presigned URL expired, 403/410, Drive 308 → re-auth, data intact (pattern #4).
    ExpiredSession,
    /// Server lost the upload session (S3 NoSuchUpload, tus 404) → reattach probe.
    SessionLost,
    /// No resume possible (one-shot POST endpoint lost the bytes).
    Unresumable,
    /// Checksum/ETag mismatch after completion.
    Corruption,
    /// Auth failure that needs user interaction (new token).
    AuthRequired,
    /// Anything we don't understand — fail fast, don't hammer.
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Retry after this delay; the chunk resumes from its confirmed offset.
    Retry(Duration),
    /// Escalate to re-auth (UI must ask the user / refresh the token).
    Reauthenticate,
    /// Reattach probe: ListParts / tus HEAD / Drive 308 continuation.
    Reattach,
    /// Give up; the journal keeps confirmed bytes for a later manual resume.
    Fail,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub backoff_base_ms: u64,
    pub backoff_cap_ms: u64,
    pub respect_retry_after: bool,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 8,
            backoff_base_ms: 500,
            backoff_cap_ms: 30_000,
            respect_retry_after: true,
        }
    }
}

impl RetryPolicy {
    /// Exponential backoff: base * 2^(attempt-1), capped.
    pub fn backoff(&self, attempt: u32) -> Duration {
        let exp = attempt.saturating_sub(1).min(16);
        let ms = self
            .backoff_base_ms
            .saturating_mul(1u64 << exp)
            .min(self.backoff_cap_ms);
        Duration::from_millis(ms)
    }

    /// Decide what to do with the Nth (1-based) failure of a chunk.
    pub fn decide(&self, kind: ErrorKind, attempt: u32, retry_after: Option<Duration>) -> Action {
        use ErrorKind::*;
        if attempt > self.max_attempts {
            return Action::Fail;
        }
        match kind {
            Transient | Corruption => Action::Retry(self.backoff(attempt)),
            Throttled => {
                if self.respect_retry_after {
                    Action::Retry(retry_after.unwrap_or_else(|| self.backoff(attempt)))
                } else {
                    Action::Retry(self.backoff(attempt))
                }
            }
            ExpiredSession => Action::Reauthenticate,
            SessionLost => Action::Reattach,
            Unresumable | AuthRequired => Action::Fail,
            Unknown => Action::Fail,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_and_caps() {
        let p = RetryPolicy::default();
        assert_eq!(p.backoff(1), Duration::from_millis(500));
        assert_eq!(p.backoff(2), Duration::from_millis(1000));
        assert_eq!(p.backoff(4), Duration::from_millis(4000));
        assert_eq!(p.backoff(20), Duration::from_millis(30_000), "capped");
    }

    #[test]
    fn decisions_follow_the_policy_table() {
        let p = RetryPolicy::default();
        assert!(matches!(
            p.decide(ErrorKind::Transient, 1, None),
            Action::Retry(_)
        ));
        assert!(
            matches!(p.decide(ErrorKind::Throttled, 1, Some(Duration::from_secs(7))), Action::Retry(d) if d == Duration::from_secs(7))
        );
        assert_eq!(
            p.decide(ErrorKind::ExpiredSession, 1, None),
            Action::Reauthenticate
        );
        assert_eq!(p.decide(ErrorKind::SessionLost, 1, None), Action::Reattach);
        assert_eq!(p.decide(ErrorKind::Unresumable, 1, None), Action::Fail);
        assert_eq!(p.decide(ErrorKind::AuthRequired, 1, None), Action::Fail);
        assert_eq!(p.decide(ErrorKind::Unknown, 1, None), Action::Fail);
        assert_eq!(
            p.decide(ErrorKind::Transient, p.max_attempts + 1, None),
            Action::Fail
        );
    }
}
