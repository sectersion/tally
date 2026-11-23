//! Moves claims whose interrupt window has closed under the horizon into the ledger.
use crate::store::{Store, StoreError};
use tally_scoring::ruleset::INTERRUPT_FOLLOW_MS;
use tally_scoring::{chain, score, GENESIS_HASH};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FinalizeSummary {
    pub appended: usize,
    /// Ledger length after the call.
    pub final_total: usize,
    /// Claims from `score()` not yet final.
    pub provisional: usize,
}

#[derive(thiserror::Error, Debug)]
pub enum FinalizeError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("ledger diverges from re-scored claims at index {index} (action {action_id})")]
    Divergence { index: usize, action_id: String },
}

/// A claim is final iff `ts + INTERRUPT_FOLLOW_MS <= horizon`. The stored ledger must be a
/// prefix of the re-scored claims, compared by `(action_id, ts, verified_s, unverified_s)`;
/// `project` is not compared, so naming a project later does not diverge the past.
pub fn finalize(store: &mut Store) -> Result<FinalizeSummary, FinalizeError> {
    // Read order matters: horizon first. Every event with ts <= h was committed before the
    // watermark reached h, so events read after the horizon are a superset of what h covers,
    // and claims final under h depend only on events <= h. Reading events first could score
    // a snapshot missing an event that a newer horizon already covers.
    let horizon = store.horizon()?;
    let mut claims = score(&store.events()?);
    let names = store.project_names()?;
    for c in &mut claims {
        if let Some(name) = c.project.as_ref().and_then(|p| names.get(p)) {
            c.project = Some(name.clone());
        }
    }
    let final_count = horizon.map_or(0, |h| {
        claims.iter().take_while(|c| c.ts.saturating_add(INTERRUPT_FOLLOW_MS) <= h).count()
    });

    let ledger = store.ledger()?;
    for (index, e) in ledger.iter().enumerate() {
        let same = claims.get(index).is_some_and(|c| {
            (&c.action_id, c.ts, c.verified_s, c.unverified_s) == (&e.action_id, e.ts, e.verified_s, e.unverified_s)
        });
        if !same {
            return Err(FinalizeError::Divergence { index, action_id: e.action_id.clone() });
        }
    }

    let new = claims.get(ledger.len()..final_count).unwrap_or(&[]);
    let entries = chain(new, ledger.last().map_or(GENESIS_HASH, |e| e.hash.as_str()));
    if let Some(h) = horizon {
        store.append_ledger(&entries, h)?;
    }
    let final_total = ledger.len() + entries.len();
    Ok(FinalizeSummary {
        appended: entries.len(),
        final_total,
        provisional: claims.len().saturating_sub(final_total),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    use crate::test_support::{edit, prompt};
    use tally_scoring::{Event, Kind, GENESIS_HASH};

    #[test] fn final_prefix_boundary_is_exact() {
        let mut s = Store::open_in_memory().unwrap();
        s.append_events(&[prompt("a", 0, 600), edit("b", 100_000)]).unwrap();
        s.set_watermark("src", 219_999).unwrap();
        let r = finalize(&mut s).unwrap();
        assert_eq!((r.appended, r.final_total, r.provisional), (1, 1, 1));   // b needs 220_000
        s.set_watermark("src", 220_000).unwrap();
        let r = finalize(&mut s).unwrap();
        assert_eq!((r.appended, r.final_total, r.provisional), (1, 2, 0));
    }
    #[test] fn nothing_is_final_without_sources_and_horizon_is_the_slowest_source() {
        let mut s = Store::open_in_memory().unwrap();
        s.append_events(&[prompt("a", 0, 600)]).unwrap();
        assert_eq!(finalize(&mut s).unwrap().appended, 0);
        s.set_watermark("fast", 10_000_000).unwrap();
        s.set_watermark("slow", 100_000).unwrap();                 // 0 + 120_000 > 100_000
        assert_eq!(finalize(&mut s).unwrap().appended, 0);
        s.set_watermark("slow", 120_000).unwrap();
        assert_eq!(finalize(&mut s).unwrap().appended, 1);
    }
    #[test] fn finalizing_twice_changes_nothing() {
        let mut s = Store::open_in_memory().unwrap();
        s.append_events(&[prompt("a", 0, 600), edit("b", 100_000)]).unwrap();
        s.set_watermark("src", 1_000_000).unwrap();
        finalize(&mut s).unwrap();
        let first = s.ledger().unwrap();
        assert_eq!(finalize(&mut s).unwrap().appended, 0);
        assert_eq!(s.ledger().unwrap(), first);
    }
    #[test] fn incremental_finalization_equals_one_shot_and_chain_verifies() {
        let all = [prompt("a", 0, 600), edit("b", 100_000), prompt("c", 900_000, 600)];
        let mut inc = Store::open_in_memory().unwrap();
        inc.append_events(&all[..2]).unwrap(); inc.set_watermark("src", 300_000).unwrap(); finalize(&mut inc).unwrap();
        inc.append_events(&all[2..]).unwrap(); inc.set_watermark("src", 2_000_000).unwrap(); finalize(&mut inc).unwrap();
        let mut one = Store::open_in_memory().unwrap();
        one.append_events(&all).unwrap(); one.set_watermark("src", 2_000_000).unwrap(); finalize(&mut one).unwrap();
        assert_eq!(inc.ledger().unwrap(), one.ledger().unwrap());
        assert!(tally_scoring::verify_chain(&inc.ledger().unwrap(), GENESIS_HASH).is_ok());
    }
    #[test] fn project_ids_map_to_public_names_only_when_registered() {
        let mut s = Store::open_in_memory().unwrap();
        s.register_project("p1", "C:/r", Some("owner/repo")).unwrap();
        let mut other = edit("b", 100_000); other.project = Some("p2".into());
        let mut none = edit("c", 200_000);  none.project = None;
        s.append_events(&[prompt("a", 0, 600), other, none]).unwrap();
        s.set_watermark("src", 10_000_000).unwrap();
        finalize(&mut s).unwrap();
        let p: Vec<_> = s.ledger().unwrap().into_iter().map(|e| e.project).collect();
        assert_eq!(p, [Some("owner/repo".into()), Some("p2".into()), None]);
    }
    #[test] fn a_changed_past_is_a_divergence_and_appends_nothing() {
        let mut s = Store::open_in_memory().unwrap();
        s.append_events(&[prompt("a", 0, 600), prompt("b", 600_000, 400)]).unwrap();
        s.set_watermark("src", 1_000_000).unwrap();
        finalize(&mut s).unwrap();
        // bypass the late-event check: an idle span that would have shortened claim "b"
        let idle = Event { id: "i".into(), ts: 100_000, kind: Kind::Idle { end_ts: 590_000 }, ..prompt("x", 0, 0) };
        s.conn.execute("INSERT INTO events (id, ts, json) VALUES (?1, ?2, ?3)",
            rusqlite::params![idle.id, idle.ts, serde_json::to_string(&idle).unwrap()]).unwrap();
        let before = s.ledger().unwrap();
        assert!(matches!(finalize(&mut s), Err(FinalizeError::Divergence { index: 1, .. })));
        assert_eq!(s.ledger().unwrap(), before);
    }
    #[test] fn a_new_slower_source_lowers_the_horizon_without_undoing_anything() {
        let mut s = Store::open_in_memory().unwrap();
        s.append_events(&[prompt("a", 0, 600), edit("b", 100_000)]).unwrap();
        s.set_watermark("fast", 1_000_000).unwrap();
        finalize(&mut s).unwrap();
        let before = s.ledger().unwrap();
        s.set_watermark("slow", 50_000).unwrap();
        assert_eq!(finalize(&mut s).unwrap().appended, 0);
        assert_eq!(s.ledger().unwrap(), before);
        assert_eq!(s.finalized_through().unwrap(), 1_000_000);
    }
    #[test] fn naming_a_project_later_does_not_diverge_the_past() {
        let mut s = Store::open_in_memory().unwrap();
        s.append_events(&[prompt("a", 0, 600)]).unwrap();
        s.set_watermark("src", 1_000_000).unwrap();
        finalize(&mut s).unwrap();
        s.register_project("p1", "C:/r", Some("owner/repo")).unwrap();
        s.append_events(&[prompt("b", 2_000_000, 600)]).unwrap();
        s.set_watermark("src", 3_000_000).unwrap();
        let r = finalize(&mut s).unwrap();
        assert_eq!((r.appended, r.final_total), (1, 2));
        let p: Vec<_> = s.ledger().unwrap().into_iter().map(|e| e.project).collect();
        assert_eq!(p, [Some("p1".into()), Some("owner/repo".into())]);
    }
}
