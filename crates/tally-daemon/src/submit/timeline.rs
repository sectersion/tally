//! The events a re-deriver needs to reproduce a project's claims in a range, with other projects anonymized.
use crate::store::{Store, StoreError};
use std::collections::BTreeSet;
use tally_scoring::review::{rederive, RederiveInput, Review};
use tally_scoring::ruleset::{INTERRUPT_FOLLOW_MS, LOOP_WINDOW_MS, MAX_INTERVAL_MS, RULESET_VERSION};
use tally_scoring::{Event, Kind};

#[derive(thiserror::Error, Debug)]
pub enum TimelineError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("range ends at {to_ms} but only claims before {finalized_through} are final")]
    NotFinal { to_ms: i64, finalized_through: i64 },
    #[error("no project registered with public name {0:?}")]
    UnknownProject(String),
    #[error("re-derivation differs from the published ledger at {action_id}")]
    Mismatch { action_id: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Timeline {
    pub events: Vec<Event>,
    /// Sorted.
    pub excluded_action_ids: Vec<String>,
}

pub fn select_timeline(store: &Store, public_name: &str, from_ms: i64, to_ms: i64) -> Result<Timeline, TimelineError> {
    let local = store.local_ids_for(public_name)?;
    if local.is_empty() {
        return Err(TimelineError::UnknownProject(public_name.into()));
    }
    let finalized_through = store.finalized_through()?;
    if to_ms.saturating_add(INTERRUPT_FOLLOW_MS) > finalized_through {
        return Err(TimelineError::NotFinal { to_ms, finalized_through });
    }
    let (lo, hi) = (from_ms.saturating_sub(LOOP_WINDOW_MS), to_ms.saturating_add(INTERRUPT_FOLLOW_MS));
    let span_floor = from_ms.saturating_sub(MAX_INTERVAL_MS);
    let events: Vec<Event> = store
        .events()?
        .into_iter()
        .filter(|e| {
            (lo..=hi).contains(&e.ts)
                || (e.ts < lo && matches!(e.kind, Kind::Idle { end_ts } | Kind::AgentBusy { end_ts } if end_ts > span_floor))
        })
        .map(|mut e| {
            if e.project.as_ref().is_some_and(|p| local.contains(p)) {
                e.project = Some(public_name.into());
            } else {
                e.project = None;
                e.session = None;
                match &mut e.kind {
                    Kind::UserEdit { file_hash, .. } => file_hash.clear(),
                    Kind::Commit { sha, .. } => sha.clear(),
                    _ => {}
                }
            }
            e
        })
        .collect();

    let run = |excluded: Vec<String>| {
        rederive(&RederiveInput {
            ruleset: RULESET_VERSION.into(),
            project: public_name.into(),
            from_ms,
            to_ms,
            events: events.clone(),
            excluded_action_ids: excluded,
            review: Review::default(),
        })
        .map_err(|_| TimelineError::Mismatch { action_id: String::new() })
    };
    let skipped: BTreeSet<String> = store.skipped()?.into_iter().map(|(id, ..)| id).collect();
    let excluded_action_ids: Vec<String> = run(vec![])?
        .claims
        .into_iter()
        .map(|c| c.action_id)
        .filter(|id| skipped.contains(id))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    let got = run(excluded_action_ids.clone())?.claims;
    // ruleset is not compared: published rows may come from an earlier chain with equal seconds
    let want: Vec<_> = store
        .published_rows(public_name)?
        .into_iter()
        .map(|r| r.entry)
        .filter(|e| (from_ms..=to_ms).contains(&e.ts))
        .collect();
    for i in 0..got.len().max(want.len()) {
        match (got.get(i), want.get(i)) {
            (Some(c), Some(w))
                if (&c.action_id, c.ts, c.verified_s, c.unverified_s) == (&w.action_id, w.ts, w.verified_s, w.unverified_s) => {}
            (c, w) => {
                let action_id = c.map(|c| c.action_id.clone()).or(w.map(|w| w.action_id.clone())).unwrap_or_default();
                return Err(TimelineError::Mismatch { action_id });
            }
        }
    }
    Ok(Timeline { events, excluded_action_ids })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::finalize::finalize;
    use crate::presence::PRESENCE_SOURCE;
    use crate::published;
    use crate::test_support::{ev, prompt};
    use tally_scoring::Actor;

    const FROM: i64 = 10_000_000;
    const TO: i64 = 12_000_000;

    fn p2(id: &str, ts: i64) -> Event {
        let mut e = ev(id, ts, Kind::UserEdit { file_hash: "secret".into(), lines_changed: 2 });
        e.project = Some("p2".into());
        e.session = Some("s2".into());
        e
    }
    fn run(id: &str, ts: i64, exit_code: i32) -> Event {
        ev(id, ts, Kind::TestRun { cmd_hash: "c".into(), exit_code, initiator: Actor::User })
    }
    fn events() -> Vec<Event> {
        let mut v = vec![
            ev("idle", FROM - 7_200_000, Kind::Idle { end_ts: FROM + 500_000 }),
            run("fail", FROM - 1_800_000, 1),
            run("pass", FROM + 600_000, 0),
        ];
        for k in 0..30 {
            v.push(prompt(&format!("a{k:02}"), FROM - 3_000_000 + k * 400_000, 600));
            v.push(p2(&format!("b{k:02}"), FROM - 2_800_000 + k * 400_000));
        }
        v
    }
    fn store(name_first: bool) -> Store {
        let mut s = Store::open_in_memory().unwrap();
        if name_first {
            s.register_project("p1", "C:/r", Some("o/one")).unwrap();
        }
        s.register_project("p2", "C:/q", None).unwrap();
        s.append_events(&events()).unwrap();
        s.set_watermark("src", 100_000_000).unwrap();
        finalize(&mut s).unwrap();
        if !name_first {
            s.register_project("p1", "C:/r", Some("o/one")).unwrap();
        }
        published::sync(&mut s, true).unwrap();
        s
    }

    #[test]
    fn timeline_rederives_to_the_ledger() {
        let s = store(true);
        let t = select_timeline(&s, "o/one", FROM, TO).unwrap();
        let got = rederive(&RederiveInput {
            ruleset: RULESET_VERSION.into(), project: "o/one".into(), from_ms: FROM, to_ms: TO,
            events: t.events.clone(), excluded_action_ids: t.excluded_action_ids.clone(), review: Review::default(),
        }).unwrap().claims;
        let want: Vec<_> = s.published_rows("o/one").unwrap().into_iter().map(|r| r.entry).filter(|e| (FROM..=TO).contains(&e.ts)).collect();
        assert!(want.len() > 3);
        assert_eq!(got.len(), want.len());
        for (g, w) in got.iter().zip(&want) {
            assert_eq!((&g.action_id, g.ts, g.verified_s, g.unverified_s, &g.reasons), (&w.action_id, w.ts, w.verified_s, w.unverified_s, &w.reasons));
        }
        assert!(got.iter().any(|c| c.reasons.iter().any(|r| r == "fail_pass_loop")));
        assert!(t.excluded_action_ids.is_empty());
    }

    #[test]
    fn named_after_finalize_still_matches() {
        let s = store(false);
        select_timeline(&s, "o/one", FROM, TO).unwrap();
    }

    #[test]
    fn skipped_claims_are_excluded_not_mismatched() {
        let mut s = Store::open_in_memory().unwrap();
        s.register_project("p1", "C:/r", Some("o/one")).unwrap();
        s.append_events(&events()).unwrap();
        s.set_presence_required(true).unwrap();
        // everything covered except a gap around a10
        let a10 = FROM - 3_000_000 + 10 * 400_000;
        s.extend_coverage(&[(0, a10 - 300_000), (a10 + 1, 100_000_000)]).unwrap();
        s.set_watermark(PRESENCE_SOURCE, 100_000_000).unwrap();
        s.set_watermark("src", 100_000_000).unwrap();
        finalize(&mut s).unwrap();
        assert!(s.skipped().unwrap().iter().any(|(id, ..)| id == "a10"));
        published::sync(&mut s, true).unwrap();
        let t = select_timeline(&s, "o/one", FROM, TO).unwrap();
        assert_eq!(t.excluded_action_ids, ["a10"]);
    }

    #[test]
    fn other_projects_are_anonymized() {
        let s = store(true);
        let t = select_timeline(&s, "o/one", FROM, TO).unwrap();
        assert!(t.events.iter().any(|e| e.id.starts_with('b')));
        for e in &t.events {
            assert_ne!(e.project.as_deref(), Some("p2"));
            assert_ne!(e.session.as_deref(), Some("s2"));
            if e.id.starts_with('b') {
                assert!(e.project.is_none());
                assert!(matches!(&e.kind, Kind::UserEdit { file_hash, .. } if file_hash.is_empty()));
            }
        }
    }

    #[test]
    fn not_final_is_refused() {
        let s = store(true);
        assert!(matches!(select_timeline(&s, "o/one", FROM, 100_000_000), Err(TimelineError::NotFinal { .. })));
    }

    #[test]
    fn unknown_public_name_is_refused() {
        let s = store(true);
        assert!(matches!(select_timeline(&s, "x/y", FROM, TO), Err(TimelineError::UnknownProject(_))));
    }
}
