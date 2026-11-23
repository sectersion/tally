//! Read-only views over the store: counts, seconds, and ledger verification.
use crate::store::{Store, StoreError};
use tally_scoring::{score, verify_chain, GENESIS_HASH};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub events: usize,
    pub final_claims: usize,
    pub provisional_claims: usize,
    pub verified_s: u64,
    pub unverified_s: u64,
    pub skipped_claims: usize,
    pub horizon: Option<i64>,
    /// `None` until the first finalization.
    pub finalized_through: Option<i64>,
    pub last_hash: String,
    /// Presence is required but has no watermark, so nothing can be final.
    pub blocked_on_presence: bool,
}

/// All reads happen in one read transaction, so counts and ledger sums always agree.
pub fn status(store: &Store) -> Result<Status, StoreError> {
    status_with(store, &mut || {})
}

/// `after_first_read` runs once the snapshot is pinned (tests commit a concurrent write there).
fn status_with(store: &Store, after_first_read: &mut dyn FnMut()) -> Result<Status, StoreError> {
    let _tx = crate::snapshot::ReadTx::begin(&store.conn)?;
    // Same read order as finalize: horizon before events.
    let horizon = store.horizon()?;
    after_first_read();
    let events = store.events()?;
    let ledger = store.ledger()?;
    let skipped = store.skipped()?.len();
    let finalized_through = Some(store.finalized_through()?).filter(|&t| t != i64::MIN);
    Ok(Status {
        events: events.len(),
        final_claims: ledger.len(),
        provisional_claims: score(&events).len().saturating_sub(ledger.len() + skipped),
        verified_s: ledger.iter().map(|e| u64::from(e.verified_s)).sum(),
        unverified_s: ledger.iter().map(|e| u64::from(e.unverified_s)).sum(),
        skipped_claims: skipped,
        horizon,
        finalized_through,
        blocked_on_presence: store.presence_required()? && store.watermark(crate::presence::PRESENCE_SOURCE)?.is_none(),
        last_hash: ledger.last().map_or(GENESIS_HASH, |e| e.hash.as_str()).to_string(),
    })
}

#[derive(thiserror::Error, Debug)]
pub enum VerifyError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("ledger broken at entry {index}: {reason}")]
    Chain { index: usize, reason: &'static str },
}

/// Number of entries when the whole chain verifies from `GENESIS_HASH`.
pub fn verify_ledger(store: &Store) -> Result<usize, VerifyError> {
    let ledger = store.ledger()?;
    verify_chain(&ledger, GENESIS_HASH).map_err(|e| VerifyError::Chain { index: e.index, reason: e.reason })?;
    Ok(ledger.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::finalize::finalize;
    use crate::test_support::{edit, prompt};

    fn store_with_two_final_claims() -> Store {
        let mut s = Store::open_in_memory().unwrap();
        s.append_events(&[prompt("a", 0, 600), edit("b", 100_000)]).unwrap();
        s.set_watermark("src", 1_000_000).unwrap();
        finalize(&mut s).unwrap();
        s
    }

    #[test] fn status_counts_final_and_provisional_claims() {
        let mut s = store_with_two_final_claims();
        s.append_events(&[edit("c", 2_000_000)]).unwrap();         // not final yet: watermark is 1_000_000
        let st = status(&s).unwrap();
        assert_eq!((st.events, st.final_claims, st.provisional_claims), (3, 2, 1));
        assert_eq!(st.verified_s, 220);                            // 120 (first action) + 100 (b)
        assert_eq!((st.horizon, st.finalized_through), (Some(1_000_000), Some(1_000_000)));
        assert_eq!(st.last_hash, s.ledger().unwrap()[1].hash);
    }
    #[test] fn status_is_one_snapshot() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("t.db");
        let mut a = Store::open(&path).unwrap();
        a.append_events(&[prompt("a", 0, 600), edit("b", 100_000)]).unwrap();
        a.set_watermark("src", 1_000_000).unwrap();
        let b = Store::open_read_only(&path).unwrap();
        let mut finalized = false;
        let st = status_with(&b, &mut || {
            // a concurrent writer finalizes between the reader's first and later reads
            finalize(&mut a).unwrap();
            finalized = true;
        }).unwrap();
        assert!(finalized);
        assert_eq!((st.final_claims, st.verified_s, st.finalized_through), (0, 0, None));
        assert_eq!(status(&b).unwrap().final_claims, 2);
    }
    #[test] fn provisional_claims_exclude_skipped_ones() {
        let mut s = store_with_two_final_claims();
        s.append_events(&[edit("c", 2_000_000)]).unwrap();
        s.append_ledger(&[], 1_000_000, 2, &[("c".into(), 2_000_000, "no_presence_coverage".into())]).unwrap();
        let st = status(&s).unwrap();
        assert_eq!((st.provisional_claims, st.skipped_claims), (0, 1));
    }
    #[test] fn status_flags_a_required_presence_without_a_watermark() {
        let mut s = Store::open_in_memory().unwrap();
        assert!(!status(&s).unwrap().blocked_on_presence);
        s.set_presence_required(true).unwrap();
        assert!(status(&s).unwrap().blocked_on_presence);
        s.set_watermark(crate::presence::PRESENCE_SOURCE, 1).unwrap();
        assert!(!status(&s).unwrap().blocked_on_presence);
    }
    #[test] fn empty_store_status() {
        let st = status(&Store::open_in_memory().unwrap()).unwrap();
        assert_eq!((st.events, st.final_claims, st.horizon, st.finalized_through), (0, 0, None, None));
        assert_eq!(st.last_hash, GENESIS_HASH);
    }
    #[test] fn verify_passes_on_an_untouched_ledger_and_names_the_tampered_entry() {
        let s = store_with_two_final_claims();
        assert_eq!(verify_ledger(&s).unwrap(), 2);
        // tamper: drop every trigger (they exist to stop this), then change entry 2's seconds
        let names: Vec<String> = s.conn.prepare("SELECT name FROM sqlite_master WHERE type='trigger'").unwrap()
            .query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
        for n in names { s.conn.execute(&format!("DROP TRIGGER {n}"), []).unwrap(); }
        let json = |s: &Store| -> String {
            s.conn.query_row("SELECT entry_json FROM ledger WHERE seq = 2", [], |r| r.get(0)).unwrap()
        };
        assert!(json(&s).contains("\"verified_s\":100"));
        s.conn.execute("UPDATE ledger SET entry_json = replace(entry_json, '\"verified_s\":100', '\"verified_s\":900') WHERE seq = 2", []).unwrap();
        assert!(!json(&s).contains("\"verified_s\":100"));
        assert!(matches!(verify_ledger(&s), Err(VerifyError::Chain { index: 1, reason: "bad_hash" })));
    }
}
