//! One tick of the daemon: poll sources, persist, finalize, run hooks.
use crate::finalize::{finalize, FinalizeSummary};
use crate::source::Source;
use crate::store::{Store, StoreError};

#[derive(thiserror::Error, Debug)]
#[error("{0}")]
pub struct HookError(pub String);

/// Work done once per tick after `finalize` (plan 05: daily publish, plan 07: sync). Gets the store and `now_ms`.
pub type TickHook = Box<dyn FnMut(&mut Store, i64) -> Result<(), HookError>>;

#[derive(Debug, Default)]
pub struct TickReport {
    pub added: usize,
    pub duplicates: usize,
    pub errors: Vec<String>,
    pub finalized: Option<FinalizeSummary>,
}

pub fn run_once(store: &mut Store, sources: &mut [Box<dyn Source>], hooks: &mut [TickHook], now_ms: i64) -> TickReport {
    let mut r = TickReport::default();
    for src in sources.iter_mut() {
        let id = src.id().to_string();
        let mut st = match store.get_state(&id) {
            Ok(st) => st,
            Err(e) => {
                r.errors.push(format!("source {id}: {e}"));
                continue;
            }
        };
        let p = match src.poll(now_ms, &mut st) {
            Ok(p) => p,
            Err(e) => {
                r.errors.push(format!("source {id}: {e}"));
                continue;
            }
        };
        // Events go in before state, so a crash in between only causes a re-read that id dedup absorbs.
        match store.append_events(&p.events) {
            Ok(s) => {
                r.added += s.added;
                r.duplicates += s.duplicates;
            }
            Err(StoreError::LateEvent { .. }) => {
                r.errors.push(format!("source {id}: dropped {} late events", p.events.len()));
            }
            Err(e) => {
                r.errors.push(format!("source {id}: {e}"));
                continue;
            }
        }
        if let Some(w) = p.watermark {
            if let Err(e) = store.set_watermark(&id, w) {
                r.errors.push(format!("source {id}: {e}"));
                continue;
            }
        }
        if let Err(e) = store.set_state(&id, &st) {
            r.errors.push(format!("source {id}: {e}"));
        }
    }
    match finalize(store) {
        Ok(f) => r.finalized = Some(f),
        Err(e) => r.errors.push(e.to_string()),
    }
    for hook in hooks.iter_mut() {
        if let Err(e) = hook(store, now_ms) {
            r.errors.push(format!("hook: {e}"));
        }
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::{Poll, SourceError, SourceState};
    use crate::test_support::prompt;
    use std::collections::VecDeque;

    struct Fake {
        id: &'static str,
        script: VecDeque<Result<Poll, SourceError>>,
        polls: usize,
    }
    impl Fake {
        fn new(id: &'static str, script: Vec<Result<Poll, SourceError>>) -> Self {
            Fake { id, script: script.into(), polls: 0 }
        }
    }
    impl Source for Fake {
        fn id(&self) -> &str {
            self.id
        }
        fn poll(&mut self, _now: i64, state: &mut dyn SourceState) -> Result<Poll, SourceError> {
            self.polls += 1;
            state.set("polls", &self.polls.to_string());
            self.script.pop_front().unwrap()
        }
    }

    #[test]
    fn a_failing_source_does_not_stop_the_others() {
        let mut s = Store::open_in_memory().unwrap();
        let mut srcs: Vec<Box<dyn Source>> = vec![
            Box::new(Fake::new("bad", vec![Err(SourceError::Other("boom".into()))])),
            Box::new(Fake::new("good", vec![Ok(Poll { events: vec![prompt("a", 0, 600)], watermark: Some(500_000) })])),
        ];
        let r = run_once(&mut s, &mut srcs, &mut [], 0);
        assert_eq!(r.added, 1);
        assert_eq!(r.errors, ["source bad: boom"]);
        assert!(s.get_state("bad").unwrap().is_empty());
        assert_eq!(s.get_state("good").unwrap()["polls"], "1");
        assert_eq!(s.horizon().unwrap(), Some(500_000));
        assert_eq!(r.finalized.unwrap().final_total, 1);
    }
    #[test]
    fn no_watermark_leaves_the_horizon_alone() {
        let mut s = Store::open_in_memory().unwrap();
        let mut srcs: Vec<Box<dyn Source>> = vec![Box::new(Fake::new("x", vec![Ok(Poll { events: vec![], watermark: None })]))];
        run_once(&mut s, &mut srcs, &mut [], 0);
        assert_eq!(s.horizon().unwrap(), None);
    }
    #[test]
    fn late_batch_is_dropped_but_state_moves_on() {
        let mut s = Store::open_in_memory().unwrap();
        s.append_events(&[prompt("old", 0, 600)]).unwrap();
        s.set_watermark("other", 1_000_000).unwrap();
        crate::finalize::finalize(&mut s).unwrap();
        let mut srcs: Vec<Box<dyn Source>> = vec![Box::new(Fake::new(
            "new",
            vec![Ok(Poll { events: vec![prompt("late", 10, 600)], watermark: Some(2_000_000) })],
        ))];
        let r = run_once(&mut s, &mut srcs, &mut [], 0);
        assert_eq!(r.errors, ["source new: dropped 1 late events"]);
        assert_eq!(s.get_state("new").unwrap()["polls"], "1");
        assert_eq!(s.events().unwrap().len(), 1);
    }
    #[test]
    fn a_failing_hook_does_not_stop_the_next() {
        let mut s = Store::open_in_memory().unwrap();
        let seen = std::rc::Rc::new(std::cell::RefCell::new(vec![]));
        let (a, b) = (seen.clone(), seen.clone());
        let mut hooks: Vec<TickHook> = vec![
            Box::new(move |_, now| {
                a.borrow_mut().push(now);
                Err(HookError("nope".into()))
            }),
            Box::new(move |_, now| {
                b.borrow_mut().push(now);
                Ok(())
            }),
        ];
        let r = run_once(&mut s, &mut [], &mut hooks, 7);
        assert_eq!(r.errors, ["hook: nope"]);
        assert_eq!(*seen.borrow(), [7, 7]);
        assert!(r.finalized.is_some());
    }
}
