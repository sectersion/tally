//! Read-only diagnosis of a ledger that no longer matches a fresh derivation. Never writes.
//!
//! Design of the (not yet available) repair, so the reason codes and messages are fixed now:
//! entries already in the ledger, the published branch or the server are never rewritten.
//! Otherwise the current chain is closed and a new epoch starts at `GENESIS_HASH`, scoring only
//! events after `finalized_through`. That needs plan 10 Task 1b (`ledger_chains.epoch`) and
//! server epoch support (plan 06 proposal: `epoch` query parameter,
//! `POST /v1/ledger/{project}/epochs`); see docs/superpowers/plans/2026-11-23-07-daemon-sync.md Task 9.
use crate::finalize::{self, FinalizeError};
use crate::published::{self, PublishError};
use crate::store::{Store, StoreError};

#[derive(Debug)]
pub enum Diagnosis {
    Clean,
    ScoringChanged { index: usize, action_id: String, field: &'static str },
    Diverged { index: usize, action_id: String, field: &'static str },
    Published(PublishError),
}

/// Fixed codes for the closing reason; never free text, since it leaves the machine.
#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    ScoringChanged,
    StoreDamaged,
    EventsEdited,
    Unknown,
}

pub const NEW_EPOCH_UNAVAILABLE: &str = "not available: needs plan 10 Task 1b (local epochs) merged and server epoch support (plan 06 proposal)";

pub fn diagnose(store: &Store) -> Result<Diagnosis, StoreError> {
    match finalize::check_derivation(store) {
        Ok(()) => {}
        Err(FinalizeError::Store(e)) => return Err(e),
        Err(FinalizeError::Divergence { index, action_id, field }) => {
            // Same seconds and a different kind, reasons or features: the scorer changed, not the data.
            let same_seconds = finalize::rescored_seconds(store, index)?
                .zip(store.ledger()?.get(index))
                .is_some_and(|(s, e)| s == (e.verified_s, e.unverified_s));
            return Ok(if same_seconds && matches!(field, "kind" | "reasons" | "features") {
                Diagnosis::ScoringChanged { index, action_id, field }
            } else {
                Diagnosis::Diverged { index, action_id, field }
            });
        }
        // A ruleset change is not a divergence of this ledger's data.
        Err(FinalizeError::RulesetChanged { .. } | FinalizeError::Busy) => {}
    }
    Ok(match published::check(store) {
        Ok(()) => Diagnosis::Clean,
        Err(PublishError::Store(e)) => return Err(e),
        Err(e) => Diagnosis::Published(e),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::finalize::finalize;
    use crate::test_support::{edit, prompt};
    use tally_scoring::{Event, Kind};

    fn finalized() -> Store {
        let mut s = Store::open_in_memory().unwrap();
        s.append_events(&[prompt("a", 0, 600), edit("b", 100_000)]).unwrap();
        s.set_watermark("src", 1_000_000).unwrap();
        finalize(&mut s).unwrap();
        s
    }
    fn drop_triggers(s: &Store) {
        let names: Vec<String> = s.conn.prepare("SELECT name FROM sqlite_master WHERE type='trigger'").unwrap()
            .query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
        for n in names { s.conn.execute(&format!("DROP TRIGGER {n}"), []).unwrap(); }
    }

    #[test] fn clean_store_is_clean() {
        assert!(matches!(diagnose(&finalized()).unwrap(), Diagnosis::Clean));
    }
    #[test] fn a_changed_feature_is_scoring_changed() {
        let s = finalized();
        let mut e = s.ledger().unwrap().remove(0);
        let key = e.features.keys().next().unwrap().clone();
        *e.features.get_mut(&key).unwrap() += 1;
        e.hash = tally_scoring::ledger::entry_hash(&e);
        drop_triggers(&s);
        s.conn.execute("UPDATE ledger SET hash = ?1, entry_json = ?2 WHERE seq = 1",
            rusqlite::params![e.hash, serde_json::to_string(&e).unwrap()]).unwrap();
        assert!(matches!(diagnose(&s).unwrap(), Diagnosis::ScoringChanged { index: 0, field: "features", .. }));
    }
    #[test] fn an_injected_idle_span_is_diverged() {
        let mut s = Store::open_in_memory().unwrap();
        s.append_events(&[prompt("a", 0, 600), prompt("b", 600_000, 400)]).unwrap();
        s.set_watermark("src", 1_000_000).unwrap();
        finalize(&mut s).unwrap();
        let idle = Event { id: "i".into(), ts: 100_000, kind: Kind::Idle { end_ts: 590_000 }, ..prompt("x", 0, 0) };
        s.conn.execute("INSERT INTO events (id, ts, json) VALUES (?1, ?2, ?3)",
            rusqlite::params![idle.id, idle.ts, serde_json::to_string(&idle).unwrap()]).unwrap();
        assert!(matches!(diagnose(&s).unwrap(), Diagnosis::Diverged { .. }));
    }
}
