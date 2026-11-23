//! The contract every event source implements (contract C1).
//!
//! Source authoring notes:
//! - A source that first appears with a watermark below `finalized_through` has promised nothing
//!   about the past. Any event it emits at or before `finalized_through` is rejected by
//!   `append_events` with `LateEvent` and is lost. The runner drops only the late events, stores the
//!   rest of the batch, still persists the source's state (so the source does not resend them
//!   forever), and prints `source <id>: dropped <n> late events`.
//! - Sources never call `finalize` and never touch the store; they read and write cursors only
//!   through `SourceState`. Extra per-source data (coverage, health, agent writes) goes through
//!   default trait methods that the runner in `run.rs` persists.
//! - Event ids must be derived from the log, never random, so a re-read is idempotent.
use std::collections::BTreeMap;

/// Adapter health for `tally doctor`; drained from a source with `take_health` and persisted by the runner.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Health {
    pub files_found: usize,
    pub records: usize,
    pub skipped: usize,
    pub errors: usize,
    /// "unknown" when the log format is not recognized.
    pub format: String,
    pub last_event_ts: Option<i64>,
    /// Source-specific counters, e.g. heartbeat accepted/dropped_agent.
    pub counters: Vec<(String, u64)>,
}

pub struct Poll {
    pub events: Vec<tally_scoring::Event>,
    pub watermark: Option<i64>,
}

#[derive(thiserror::Error, Debug)]
pub enum SourceError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Other(String),
}

pub trait SourceState {
    fn get(&self, key: &str) -> Option<String>;
    fn set(&mut self, key: &str, value: &str);
}

impl SourceState for BTreeMap<String, String> {
    fn get(&self, key: &str) -> Option<String> {
        BTreeMap::get(self, key).cloned()
    }
    fn set(&mut self, key: &str, value: &str) {
        self.insert(key.to_string(), value.to_string());
    }
}

pub trait Source {
    fn id(&self) -> &str;
    /// Returns new events plus an optional watermark: a promise that this source will never again
    /// emit an event with `ts <=` the watermark. The runner raises the horizon to the minimum
    /// watermark across sources, and claims under it can finalize. `None` leaves the promise as is.
    /// A returned watermark must be below the start of every open `agent_busy` span, and below the
    /// timestamp of every line not yet read in full (a partial last line included).
    /// Cursor changes made through `state` are persisted only if `poll` returns `Ok` and the runner
    /// then succeeds in `append_events` (late events are dropped alone: the rest and the state are still saved) and
    /// in `set_watermark`; on any other store failure nothing is persisted and the next tick retries.
    fn poll(&mut self, now_ms: i64, state: &mut dyn SourceState) -> Result<Poll, SourceError>;
    /// Drains the coverage produced by the last `poll`. The runner stores it before the watermark
    /// moves, so finalize never sees a horizon that outruns coverage.
    fn take_coverage(&mut self) -> Vec<crate::store::Interval> {
        Vec::new()
    }
    /// Drains the health of the last poll. `None` = unchanged. The runner persists it.
    fn take_health(&mut self) -> Option<Health> {
        None
    }
    /// Drains agent file writes seen by the last poll as `(file_hash, ts)`. Never an event.
    fn take_agent_writes(&mut self) -> Vec<(String, i64)> {
        Vec::new()
    }
    /// Receives every agent write drained from all sources in this tick (after all polls).
    fn see_agent_writes(&mut self, _writes: &[(String, i64)]) {}
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake;
    impl Source for Fake {
        fn id(&self) -> &str {
            "fake"
        }
        fn poll(&mut self, _: i64, _: &mut dyn SourceState) -> Result<Poll, SourceError> {
            Ok(Poll { events: vec![], watermark: None })
        }
    }

    #[test]
    fn default_take_methods_are_empty() {
        let mut f = Fake;
        assert_eq!(f.take_health(), None);
        assert!(f.take_agent_writes().is_empty());
    }
}
