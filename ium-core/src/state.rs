//! Supervisor state machine (spec §3.1) — the upload twin of Motrix's
//! engine-supervisor pattern. Every transition is journaled by the caller
//! BEFORE it executes, so a crash on any arrow resumes at the same arrow.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Created,
    Probing,
    Running,
    Paused,
    Finishing,
    Done,
    Unresumable,
    Failed,
}

/// Why a job left the happy path (kept in the journal payload).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    Cancelled,
    ExpiredSession,
    ProtocolViolation,
    Unknown,
}

#[derive(Debug, thiserror::Error)]
#[error("{from:?} -> {to:?} is not a legal transition")]
pub struct StateError {
    pub from: JobState,
    pub to: JobState,
}

impl JobState {
    /// Legal transitions (spec §3.1 diagram).
    pub fn transition(self, to: JobState) -> Result<JobState, StateError> {
        use JobState::*;
        let legal = matches!(
            (self, to),
            (Created, Probing)
                | (Created, Failed) // rejected before probing (e.g. unknown adapter)
                | (Probing, Running)
                | (Probing, Unresumable) // one-shot endpoint + lost session
                | (Probing, Paused) // user paused during probe
                | (Probing, Failed)
                | (Running, Paused)
                | (Running, Finishing)
                | (Running, Failed)
                | (Paused, Running)
                | (Paused, Failed)
                | (Finishing, Done)
                | (Finishing, Running) // verify failed → repair loop
                | (Finishing, Failed)
        );
        if legal {
            Ok(to)
        } else {
            Err(StateError { from: self, to })
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            JobState::Done | JobState::Failed | JobState::Unresumable
        )
    }

    pub fn as_str(self) -> &'static str {
        match self {
            JobState::Created => "created",
            JobState::Probing => "probing",
            JobState::Running => "running",
            JobState::Paused => "paused",
            JobState::Finishing => "finishing",
            JobState::Done => "done",
            JobState::Unresumable => "unresumable",
            JobState::Failed => "failed",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn happy_path_is_legal() {
        let mut s = JobState::Created;
        for next in [
            JobState::Probing,
            JobState::Running,
            JobState::Finishing,
            JobState::Done,
        ] {
            s = s.transition(next).unwrap();
        }
        assert!(s.is_terminal());
    }

    #[test]
    fn pause_resume_loop_is_legal() {
        let s = JobState::Running.transition(JobState::Paused).unwrap();
        assert_eq!(s.transition(JobState::Running).unwrap(), JobState::Running);
    }

    #[test]
    fn verify_failure_reenters_running() {
        let s = JobState::Finishing.transition(JobState::Running).unwrap();
        assert_eq!(s, JobState::Running);
    }

    #[test]
    fn illegal_jumps_are_rejected() {
        assert!(JobState::Created.transition(JobState::Done).is_err());
        assert!(JobState::Done.transition(JobState::Running).is_err());
        assert!(JobState::Unresumable.transition(JobState::Running).is_err());
        assert!(JobState::Paused.transition(JobState::Done).is_err());
        assert!(JobState::Failed.transition(JobState::Running).is_err());
    }

    #[test]
    fn serde_roundtrip_is_stable_for_journal() {
        for s in [
            JobState::Created,
            JobState::Probing,
            JobState::Running,
            JobState::Paused,
            JobState::Finishing,
            JobState::Done,
            JobState::Unresumable,
            JobState::Failed,
        ] {
            let json = serde_json::to_string(&s).unwrap();
            assert_eq!(serde_json::from_str::<JobState>(&json).unwrap(), s);
            assert_eq!(json, format!("\"{}\"", s.as_str()));
        }
    }
}
