//! The contract every event source implements (contract C1).
//!
//! Source authoring notes:
//! - A source that first appears with a watermark below `finalized_through` has promised nothing
//!   about the past. Any event it emits at or before `finalized_through` is rejected by
//!   `append_events` with `LateEvent`, and the batch is lost. The runner then drops that batch but
//!   still persists the source's state, so the source does not resend it forever, and prints
//!   `source <id>: dropped <n> late events`.
//! - Sources never call `finalize` and never touch the store; they read and write cursors only
//!   through `SourceState`. Extra per-source data (coverage, health, agent writes) goes through
//!   default trait methods that the runner in `run.rs` persists.
//! - Event ids must be derived from the log, never random, so a re-read is idempotent.
use std::collections::BTreeMap;

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
    /// then succeeds in `append_events` (a late batch is the exception: state is still saved) and
    /// in `set_watermark`; on any other store failure nothing is persisted and the next tick retries.
    fn poll(&mut self, now_ms: i64, state: &mut dyn SourceState) -> Result<Poll, SourceError>;
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
