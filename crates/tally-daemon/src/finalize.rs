//! Moves claims whose interrupt window has closed under the horizon into the ledger.
use crate::presence::PRESENCE_SOURCE;
use crate::store::{Store, StoreError};
use std::collections::HashSet;
use tally_scoring::ruleset::{FIRST_ACTION_MS, INTERRUPT_FOLLOW_MS, MAX_INTERVAL_MS};
use tally_scoring::{chain, score, LedgerEntry, GENESIS_HASH};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FinalizeSummary {
    pub appended: usize,
    /// Claims newly skipped by this call.
    pub skipped: usize,
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
    #[error("ledger was written under ruleset {ledger} but this build scores with {current}; finalize will not append across a ruleset change")]
    RulesetChanged { ledger: String, current: String },
}

/// A claim is final iff `ts + INTERRUPT_FOLLOW_MS <= horizon`. The stored ledger must be a
/// prefix of the re-scored claims, compared by `(action_id, ts, verified_s, unverified_s)`;
/// `project` is not compared, so naming a project later does not diverge the past.
///
/// The horizon can go down when a new, slower source registers. Claims already ledgered are
/// never withdrawn and `finalized_through` never moves back; later claims wait for the new
/// horizon. If another writer adds an event at or below the horizon while this runs (a new
/// source with no watermark yet), the append is refused with `Conflict` and everything is
/// re-read; up to 3 attempts in total on `Conflict` or `ChainBroken`.
///
/// Presence guard ("live capture only"), active iff `store.presence_required()`; off, no
/// claim is ever skipped:
/// - With no `PRESENCE_SOURCE` watermark the horizon is `None`: nothing is final (`tally run`
///   tells the user; nothing is silently skipped).
/// - A claim is decided once, when final. Its interval is `[start, ts]`, `start` being the
///   previous claim's ts (of the whole list, skipped or not: the previous human action) if
///   within `MAX_INTERVAL_MS`, else `ts - FIRST_ACTION_MS`. It is ledgered iff one stored
///   coverage interval `(s, e)` has `s <= start && ts <= e`, else skipped with reason
///   `no_presence_coverage`, in full, never trimmed. Time before the daemon first ran, or
///   while it was off or the machine slept, is outside coverage.
/// - Safe because the presence runner writes coverage before advancing its watermark and
///   this reads the horizon before the coverage: every final claim's coverage is already
///   stored when it is decided. Decisions are never revisited, so later coverage cannot
///   resurrect a skip nor make a ledgered claim diverge.
/// - Ledger and skips are written in one `append_ledger` transaction; the ledger is a hash
///   chain over credited claims only. Divergence compares the ledger with the re-scored
///   claims minus every recorded skip (exact: skips were never ledgered). Undecided claims
///   are a suffix of the claim list, since decisions go in time order.
pub fn finalize(store: &mut Store) -> Result<FinalizeSummary, FinalizeError> {
    retry(|| {
        let p = prepare(store)?;
        commit(store, p)
    })
}

const ATTEMPTS: usize = 3;

fn retry<T>(mut attempt: impl FnMut() -> Result<T, FinalizeError>) -> Result<T, FinalizeError> {
    let mut n = 1;
    loop {
        match attempt() {
            Err(FinalizeError::Store(StoreError::Conflict | StoreError::ChainBroken)) if n < ATTEMPTS => n += 1,
            r => return r,
        }
    }
}

struct Prepared {
    horizon: Option<i64>,
    /// Events with `ts <= horizon` that were scored.
    scored_events: usize,
    /// Skipped claims stored when read; `append_ledger_checked` refuses if it changed.
    skipped_seen: usize,
    entries: Vec<LedgerEntry>,
    skips: Vec<(String, i64, String)>,
    summary: FinalizeSummary,
}

fn prepare(store: &Store) -> Result<Prepared, FinalizeError> {
    // Read order matters: horizon first. Every event with ts <= h was committed before the
    // watermark reached h, so events read after the horizon are a superset of what h covers,
    // and claims final under h depend only on events <= h. Reading events first could score
    // a snapshot missing an event that a newer horizon already covers. A source with no
    // watermark yet is outside that argument; `append_ledger`'s count check covers it.
    let guard = store.presence_required()?;
    let horizon = if guard && store.watermark(PRESENCE_SOURCE)?.is_none() { None } else { store.horizon()? };
    let events = store.events()?;
    let scored_events = horizon.map_or(0, |h| events.iter().filter(|e| e.ts <= h).count());
    let mut claims = score(&events);
    let names = store.project_names()?;
    for c in &mut claims {
        if let Some(name) = c.project.as_ref().and_then(|p| names.get(p)) {
            c.project = Some(name.clone());
        }
    }
    // Read after the horizon, so it holds every interval the horizon implies.
    let coverage = if guard { store.coverage()? } else { Vec::new() };
    let skipped: HashSet<String> = store.skipped()?.into_iter().map(|(id, ..)| id).collect();
    // Indexes into `claims` of every claim not already skipped.
    let kept: Vec<usize> = (0..claims.len()).filter(|&i| !skipped.contains(&claims[i].action_id)).collect();

    let ledger = store.ledger()?;
    if let Some(last) = ledger.last().filter(|e| e.ruleset != tally_scoring::RULESET_VERSION) {
        return Err(FinalizeError::RulesetChanged {
            ledger: last.ruleset.clone(),
            current: tally_scoring::RULESET_VERSION.to_string(),
        });
    }
    for (index, e) in ledger.iter().enumerate() {
        let same = kept.get(index).map(|&i| &claims[i]).is_some_and(|c| {
            (&c.action_id, c.ts, c.verified_s, c.unverified_s) == (&e.action_id, e.ts, e.verified_s, e.unverified_s)
        });
        if !same {
            return Err(FinalizeError::Divergence { index, action_id: e.action_id.clone() });
        }
    }

    let (mut new, mut skips) = (Vec::new(), Vec::new());
    for &i in kept.get(ledger.len()..).unwrap_or(&[]) {
        let c = &claims[i];
        if horizon.is_none_or(|h| c.ts.saturating_add(INTERRUPT_FOLLOW_MS) > h) {
            break;
        }
        // Same interval as the scorer: the previous claim of the whole list, skipped or not.
        let start = match i.checked_sub(1).map(|p| claims[p].ts) {
            Some(p) if c.ts.saturating_sub(p) <= MAX_INTERVAL_MS => p,
            _ => c.ts.saturating_sub(FIRST_ACTION_MS),
        };
        if !guard || coverage.iter().any(|&(s, e)| s <= start && c.ts <= e) {
            new.push(c.clone());
        } else {
            skips.push((c.action_id.clone(), c.ts, NO_COVERAGE.to_string()));
        }
    }
    let entries = chain(&new, ledger.last().map_or(GENESIS_HASH, |e| e.hash.as_str()));
    let final_total = ledger.len() + entries.len();
    let summary = FinalizeSummary {
        appended: entries.len(),
        skipped: skips.len(),
        final_total,
        provisional: claims.len().saturating_sub(final_total + skipped.len() + skips.len()),
    };
    Ok(Prepared { horizon, scored_events, skipped_seen: skipped.len(), entries, skips, summary })
}

const NO_COVERAGE: &str = "no_presence_coverage";

fn commit(store: &mut Store, p: Prepared) -> Result<FinalizeSummary, FinalizeError> {
    if let Some(h) = p.horizon {
        store.append_ledger_checked(&p.entries, h, p.scored_events, &p.skips, Some(p.skipped_seen))?;
    }
    Ok(p.summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    use crate::test_support::{edit, prompt};
    use tally_scoring::{Event, Kind, GENESIS_HASH};

    fn old_ruleset_entry() -> LedgerEntry {
        let mut e = LedgerEntry { v: 1, ruleset: "old-0".into(), action_id: "a".into(), ts: 1, project: None,
            kind: "k".into(), verified_s: 1, unverified_s: 0, reasons: vec![], features: Default::default(),
            prev_hash: GENESIS_HASH.into(), hash: String::new() };
        e.hash = tally_scoring::ledger::entry_hash(&e);
        e
    }
    #[test] fn finalize_refuses_to_append_across_a_ruleset_change() {
        let mut s = Store::open_in_memory().unwrap();
        s.append_ledger(&[old_ruleset_entry()], 5, 0, &[]).unwrap();
        s.append_events(&[prompt("a", 10, 600)]).unwrap();
        s.set_watermark("src", 10_000_000).unwrap();
        assert!(matches!(finalize(&mut s),
            Err(FinalizeError::RulesetChanged { ledger, .. }) if ledger == "old-0"));
        assert_eq!(s.ledger().unwrap().len(), 1);
        assert_eq!(s.finalized_through().unwrap(), 5);
    }
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
    #[test] fn an_event_from_a_new_source_mid_finalize_is_retried_and_matches_one_shot() {
        let dir = tempfile::tempdir().unwrap(); let path = dir.path().join("tally.db");
        let mut a = Store::open(&path).unwrap();
        let mut b = Store::open(&path).unwrap();
        a.append_events(&[prompt("a", 0, 600), edit("b", 100_000)]).unwrap();
        a.set_watermark("src", 1_000_000).unwrap();
        // the new source has no watermark yet, so it does not hold the horizon back
        let late = prompt("c", 50_000, 600);
        let mut calls = 0;
        let r = retry(|| {
            calls += 1;
            let p = prepare(&a)?;
            if calls == 1 { b.append_events(std::slice::from_ref(&late)).unwrap(); }
            commit(&mut a, p)
        }).unwrap();
        assert_eq!(calls, 2);
        let mut one = Store::open_in_memory().unwrap();
        one.append_events(&[prompt("a", 0, 600), edit("b", 100_000), late]).unwrap();
        one.set_watermark("src", 1_000_000).unwrap();
        assert_eq!(r, finalize(&mut one).unwrap());
        assert_eq!(a.ledger().unwrap(), one.ledger().unwrap());
        assert_eq!(finalize(&mut a).unwrap().appended, 0);    // no Divergence afterwards
    }
    fn guarded() -> Store {
        let mut s = Store::open_in_memory().unwrap();
        s.set_presence_required(true).unwrap();
        s
    }
    fn ids(s: &Store) -> Vec<String> { s.ledger().unwrap().into_iter().map(|e| e.action_id).collect() }
    fn skipped_ids(s: &Store) -> Vec<String> { s.skipped().unwrap().into_iter().map(|e| e.0).collect() }
    /// a, b covered; c's interval [1_100_000, 1_900_000] spans the coverage gap.
    fn gap_case(s: &mut Store) -> FinalizeSummary {
        s.append_events(&[prompt("a", 1_000_000, 600), edit("b", 1_100_000), edit("c", 1_900_000)]).unwrap();
        s.extend_coverage(&[(800_000, 1_200_000), (1_500_000, 3_000_000)]).unwrap();
        s.set_watermark(PRESENCE_SOURCE, 3_000_000).unwrap();
        finalize(s).unwrap()
    }

    #[test] fn uncovered_claim_is_skipped_with_a_reason_and_covered_ones_are_ledgered() {
        let mut s = guarded();
        let r = gap_case(&mut s);
        assert_eq!((r.appended, r.skipped, r.final_total, r.provisional), (2, 1, 2, 0));
        assert_eq!(ids(&s), ["a", "b"]);
        assert_eq!(s.skipped().unwrap(), vec![("c".to_string(), 1_900_000, "no_presence_coverage".to_string())]);
        assert!(tally_scoring::verify_chain(&s.ledger().unwrap(), GENESIS_HASH).is_ok());
    }
    #[test] fn a_skip_is_permanent_and_later_claims_still_chain() {
        let mut s = guarded();
        gap_case(&mut s);
        let before = s.ledger().unwrap();
        // coverage that would have credited c arrives late (raw insert: extend_coverage never
        // lowers a row's start, so it cannot produce this row itself)
        s.conn.execute("INSERT INTO coverage(start_ts, end_ts) VALUES (1000000, 2000000)", []).unwrap();
        let r = finalize(&mut s).unwrap();
        assert_eq!((r.appended, r.skipped), (0, 0));
        assert_eq!(s.ledger().unwrap(), before);
        assert_eq!(skipped_ids(&s), ["c"]);
        // d's interval starts at the skipped c (1_900_000), not at the last ledgered b (1_100_000):
        // only (1_500_000, 3_500_000) covers [1_900_000, 3_100_000]
        s.append_events(&[edit("d", 3_100_000)]).unwrap();
        s.extend_coverage(&[(3_000_000, 3_500_000)]).unwrap();
        s.set_watermark(PRESENCE_SOURCE, 3_300_000).unwrap();
        let r = finalize(&mut s).unwrap();
        assert_eq!((r.appended, r.skipped, r.final_total, r.provisional), (1, 0, 3, 0));
        assert_eq!(ids(&s), ["a", "b", "d"]);
        assert_eq!(s.ledger().unwrap()[..2], before[..]);
        assert!(tally_scoring::verify_chain(&s.ledger().unwrap(), GENESIS_HASH).is_ok());
        assert_eq!(finalize(&mut s).unwrap().appended, 0);    // no Divergence afterwards
    }
    #[test] fn coverage_boundary_is_inclusive_and_one_ms_short_is_skipped() {
        let x = 1_000_000;
        let first = x - FIRST_ACTION_MS;
        for (cov, credited) in [((first, x), true), ((first + 1, x), false), ((first, x - 1), false)] {
            let mut s = guarded();
            s.append_events(&[edit("x", x)]).unwrap();
            s.extend_coverage(&[cov]).unwrap();
            s.set_watermark(PRESENCE_SOURCE, x + INTERRUPT_FOLLOW_MS).unwrap();
            let r = finalize(&mut s).unwrap();
            assert_eq!((r.appended, r.skipped), if credited { (1, 0) } else { (0, 1) }, "{cov:?}");
        }
    }
    #[test] fn guard_off_changes_nothing() {
        let mut s = Store::open_in_memory().unwrap();
        let r = gap_case(&mut s);
        assert_eq!((r.appended, r.skipped, r.final_total, r.provisional), (3, 0, 3, 0));
        assert_eq!(ids(&s), ["a", "b", "c"]);
        assert!(s.skipped().unwrap().is_empty());
    }
    #[test] fn required_without_a_presence_watermark_finalizes_nothing() {
        let mut s = guarded();
        s.append_events(&[prompt("a", 1_000_000, 600)]).unwrap();
        s.extend_coverage(&[(0, 10_000_000)]).unwrap();
        s.set_watermark("other", 10_000_000).unwrap();
        let r = finalize(&mut s).unwrap();
        assert_eq!((r.appended, r.skipped, r.final_total, r.provisional), (0, 0, 0, 1));
        assert!(s.ledger().unwrap().is_empty());
        assert!(s.skipped().unwrap().is_empty());
        assert_eq!(s.finalized_through().unwrap(), i64::MIN);
    }
    #[test] fn sleep_gap_end_to_end_with_the_tracker() {
        const B: i64 = 1_000_000;
        let t = |secs: i64| B + secs * 1000;
        let mut s = guarded();
        s.append_events(&[edit("e300", t(300)), edit("e500", t(500)), edit("e1900", t(1900)), edit("e2000", t(2000))]).unwrap();
        let mut tracker = crate::presence::Tracker::default();
        let samples = (0..=600).step_by(5).chain((1800..=2400).step_by(5));
        for secs in samples {
            // the runner's order: coverage first, then the watermark, then finalize
            let st = tracker.step(t(secs), 0);
            s.extend_coverage(&st.coverage).unwrap();
            s.set_watermark(PRESENCE_SOURCE, st.watermark).unwrap();
            finalize(&mut s).unwrap();
        }
        assert_eq!(s.coverage().unwrap(), vec![(t(0), t(600)), (t(1800), t(2400))]);
        assert_eq!(ids(&s), ["e300", "e500", "e2000"]);
        assert_eq!(skipped_ids(&s), ["e1900"]);
        assert!(tally_scoring::verify_chain(&s.ledger().unwrap(), GENESIS_HASH).is_ok());
    }
    #[test] fn status_reports_skipped_claims() {
        let mut s = guarded();
        gap_case(&mut s);
        assert_eq!(crate::status::status(&s).unwrap().skipped_claims, 1);
    }
    #[test] fn a_changed_past_under_the_guard_is_still_a_divergence() {
        let mut s = guarded();
        gap_case(&mut s);
        s.append_events(&[edit("d", 3_100_000)]).unwrap();
        s.extend_coverage(&[(3_000_000, 3_500_000)]).unwrap();
        s.set_watermark(PRESENCE_SOURCE, 3_300_000).unwrap();
        finalize(&mut s).unwrap();
        assert_eq!(ids(&s), ["a", "b", "d"]);
        // an idle span inside d's interval [1_900_000, 3_100_000] shortens d's re-scored credit;
        // d is ledger index 2 but claim index 3 (the skipped c sits between b and d)
        let idle = Event { id: "i".into(), ts: 1_900_001, kind: Kind::Idle { end_ts: 3_000_000 }, ..prompt("x", 0, 0) };
        s.conn.execute("INSERT INTO events (id, ts, json) VALUES (?1, ?2, ?3)",
            rusqlite::params![idle.id, idle.ts, serde_json::to_string(&idle).unwrap()]).unwrap();
        let before = (s.ledger().unwrap(), s.skipped().unwrap());
        assert!(matches!(finalize(&mut s), Err(FinalizeError::Divergence { index: 2, ref action_id }) if action_id == "d"));
        assert_eq!((s.ledger().unwrap(), s.skipped().unwrap()), before);
    }
    #[test] fn a_skip_recorded_by_another_finalizer_mid_finalize_is_retried() {
        let dir = tempfile::tempdir().unwrap(); let path = dir.path().join("tally.db");
        let mut a = Store::open(&path).unwrap();
        let mut b = Store::open(&path).unwrap();
        a.set_presence_required(true).unwrap();
        gap_case(&mut a);
        a.append_events(&[edit("d", 3_100_000)]).unwrap();
        a.extend_coverage(&[(3_000_000, 3_500_000)]).unwrap();
        a.set_watermark(PRESENCE_SOURCE, 3_300_000).unwrap();
        let p = prepare(&a).unwrap();
        b.append_ledger(&[], 3_300_000, p.scored_events, &[("zz".into(), 1, "x".into())]).unwrap();
        assert!(matches!(commit(&mut a, p), Err(FinalizeError::Store(StoreError::Conflict))));
        assert_eq!(a.ledger().unwrap().len(), 2);
        assert_eq!(finalize(&mut a).unwrap().appended, 1);        // the retry sees the new skip and succeeds
    }
    #[test] fn retry_gives_up_after_three_attempts_and_only_on_conflict_or_chain_broken() {
        let mut calls = 0;
        let r: Result<(), _> = retry(|| { calls += 1; Err(FinalizeError::Store(StoreError::Conflict)) });
        assert!(matches!(r, Err(FinalizeError::Store(StoreError::Conflict))));
        assert_eq!(calls, 3);
        calls = 0;
        let r: Result<(), _> = retry(|| { calls += 1; Err(FinalizeError::Divergence { index: 0, action_id: "x".into() }) });
        assert!(r.is_err());
        assert_eq!(calls, 1);
    }
}
