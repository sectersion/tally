//! One tick of the daemon: poll sources, persist, finalize, run hooks.
use crate::finalize::{finalize, FinalizeSummary};
use crate::config::Config;
use crate::source::{now_ms, Source};
use crate::store::{Store, StoreError};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

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
        // A late event is dropped alone; the rest of the batch is stored (id dedup still applies).
        // Each retry removes the one event the store named, so known ids at or before finalized_through
        // still count as duplicates, not as dropped.
        // ponytail: one retry per late event (O(late x batch)); filter by finalized_through + id lookup if batches of late events get big.
        let coverage = src.take_coverage();
        let (mut events, mut dropped) = (p.events, 0);
        let appended = loop {
            match store.append_events(&events) {
                Err(StoreError::LateEvent { id: late, .. }) => {
                    events.retain(|e| e.id != late);
                    dropped += 1;
                }
                res => break res,
            }
        };
        if dropped > 0 {
            r.errors.push(format!("source {id}: dropped {dropped} late events"));
        }
        match appended {
            Ok(s) => {
                r.added += s.added;
                r.duplicates += s.duplicates;
            }
            Err(e) => {
                r.errors.push(format!("source {id}: {e}"));
                continue;
            }
        }
        // Coverage goes in before the watermark: finalize must never see a horizon past stored coverage.
        if let Err(e) = store.extend_coverage(&coverage) {
            r.errors.push(format!("source {id}: {e}"));
            continue;
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

/// The sources for this config. Claude Code reads `projects_dir`, default `~/.claude/projects`.
/// Also records `presence_required` and the config-disabled ids (the finalize horizon skips them), in one transaction, and fills
/// missing project identities; failures there are reported on stderr, not fatal.
pub fn sources(cfg: &Config, home: &Path, store: &mut Store) -> Result<Vec<Box<dyn Source>>, StoreError> {
    if let Err(e) = crate::project::backfill_identities(store) {
        eprintln!("project identity backfill failed: {e}");
    }
    let mut v: Vec<Box<dyn Source>> = vec![];
    if cfg.presence.enabled {
        v.push(Box::new(crate::presence::PresenceSource::new(crate::presence::default_platform())));
    }
    let cc = &cfg.claude_code;
    if let Some(dir) = cc.projects_dir.clone().or_else(|| dirs::home_dir().map(|h| h.join(".claude").join("projects"))) {
        if cc.enabled {
            v.push(Box::new(crate::claude_code::ClaudeCode::new(dir)));
        }
    }
    // The git source is always on (no `[git]` config); with no registered repos it polls nothing.
    // It reads agent markers through its own connection to the db file under `home`.
    match (store.repos_to_poll(), Store::open(&home.join("tally.db"))) {
        (Ok(repos), Ok(own)) => {
            let repos = repos.into_iter().map(|(local_id, root)| crate::git::RepoRef { local_id, root: root.into() }).collect();
            let lookup = crate::git::StoreLookup { store: own, claude_enabled: cc.enabled };
            v.push(Box::new(crate::git::GitSource::new(repos, Box::new(lookup))));
        }
        (repos, own) => {
            if let Err(e) = repos {
                eprintln!("git source disabled: listing repositories failed: {e}");
            }
            if let Err(e) = own {
                eprintln!("git source disabled: opening the database failed: {e}");
            }
        }
    }
    // What config switched off, not what got built: a git setup error must not drop git's watermark.
    let mut disabled = vec![];
    if !cfg.presence.enabled {
        disabled.push("presence");
    }
    if !cc.enabled {
        disabled.push("claude-code");
    }
    // Fatal: a stale presence_required would over-credit all session.
    store.set_run_config(cfg.presence.required(), &disabled)?;
    Ok(v)
}

/// The per-tick hooks: daily publish, then sync when enabled.
pub fn hooks(cfg: &Config, home: &Path) -> Vec<TickHook> {
    let publish = cfg.publish.clone();
    let h = home.to_path_buf();
    let mut scheduler = crate::publish::Scheduler::new();
    let mut v: Vec<TickHook> = vec![Box::new(move |store, now_ms| {
        scheduler.tick(store, &publish, &h, crate::publish::local_day_number(chrono::Local::now()), now_ms);
        Ok(())
    })];
    v.extend(sync_hook(cfg, home));
    v
}

/// `None` unless `[sync]` is enabled and has a server.
pub fn sync_hook(cfg: &Config, home: &Path) -> Option<TickHook> {
    let server = cfg.sync.server.clone().filter(|_| cfg.sync.enabled)?;
    let home = home.to_path_buf();
    let mut ticker = crate::sync::SyncTicker::new();
    Some(Box::new(move |store, now_ms| {
        ticker.tick(store, &server, &home, now_ms);
        Ok(())
    }))
}

/// Seconds between ticks: 0 would busy-loop and a huge value overflows `Instant`, so 1 s to 24 h.
pub fn effective_interval(secs: u64) -> u64 {
    secs.clamp(1, 86_400)
}

/// Tick every `cfg.run.interval_secs` until `stop` is set. The current tick always completes.
pub fn run_loop(store: &mut Store, cfg: &Config, home: &Path, stop: &AtomicBool) -> Result<(), StoreError> {
    let mut srcs = sources(cfg, home, store)?;
    let mut hks = hooks(cfg, home);
    while !stop.load(Ordering::Relaxed) {
        for e in run_once(store, &mut srcs, &mut hks, now_ms()).errors {
            eprintln!("{e}");
        }
        let deadline = Instant::now() + Duration::from_secs(effective_interval(cfg.run.interval_secs));
        while !stop.load(Ordering::Relaxed) {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            std::thread::sleep(left.min(Duration::from_millis(200)));
        }
    }
    Ok(())
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
    fn only_the_late_events_of_a_batch_are_dropped() {
        let mut s = Store::open_in_memory().unwrap();
        s.append_events(&[prompt("old", 0, 600)]).unwrap();
        s.set_watermark("other", 1_000_000).unwrap();
        crate::finalize::finalize(&mut s).unwrap();
        let batch = vec![prompt("late", 10, 600), prompt("old", 0, 600), prompt("fresh", 2_000_000, 600)];
        let mut srcs: Vec<Box<dyn Source>> =
            vec![Box::new(Fake::new("new", vec![Ok(Poll { events: batch, watermark: Some(3_000_000) })]))];
        let r = run_once(&mut s, &mut srcs, &mut [], 0);
        // "old" is a known id at or below finalized_through: a duplicate, not dropped
        assert_eq!(r.errors, ["source new: dropped 1 late events"]);
        assert_eq!((r.added, r.duplicates), (1, 1));
        let ids: Vec<_> = s.events().unwrap().into_iter().map(|e| e.id).collect();
        assert_eq!(ids, ["old", "fresh"]);
        assert_eq!(s.get_state("new").unwrap()["polls"], "1");
        assert_eq!(s.horizon().unwrap(), Some(1_000_000));
        let w: i64 = s.conn.query_row("SELECT ts FROM watermarks WHERE source='new'", [], |r| r.get(0)).unwrap();
        assert_eq!(w, 3_000_000);
    }
    #[test]
    fn coverage_is_stored_before_the_watermark_moves() {
        struct Cov;
        impl Source for Cov {
            fn id(&self) -> &str {
                "cov"
            }
            fn poll(&mut self, _: i64, _: &mut dyn SourceState) -> Result<Poll, SourceError> {
                Ok(Poll { events: vec![], watermark: Some(9) })
            }
            fn take_coverage(&mut self) -> Vec<crate::store::Interval> {
                vec![(1, 5)]
            }
        }
        let mut s = Store::open_in_memory().unwrap();
        // make the watermark write fail: coverage must already be stored by then
        s.conn.execute("DROP TABLE watermarks", []).unwrap();
        let mut srcs: Vec<Box<dyn Source>> = vec![Box::new(Cov)];
        let r = run_once(&mut s, &mut srcs, &mut [], 0);
        assert!(r.errors.iter().any(|e| e.starts_with("source cov:")), "{:?}", r.errors);
        assert_eq!(s.coverage().unwrap(), [(1, 5)]);
    }
    #[test]
    fn sources_registers_presence_only_when_enabled_and_git_always() {
        let d = tempfile::tempdir().unwrap();
        let mut s = Store::open(&d.path().join("tally.db")).unwrap();
        let mut cfg = Config::default();
        cfg.claude_code.enabled = false;
        cfg.presence.enabled = true;
        s.set_watermark("presence", 5).unwrap();
        s.set_watermark("git", 100).unwrap();
        let mut ids = |c: &Config| sources(c, d.path(), &mut s).unwrap().iter().map(|x| x.id().to_string()).collect::<Vec<_>>();
        assert_eq!(ids(&cfg), ["presence", "git"]);
        cfg.presence.enabled = false;
        assert_eq!(ids(&cfg), ["git"]);
        // the disabled presence watermark no longer holds the horizon
        assert_eq!(s.horizon().unwrap(), Some(100));
        cfg.presence.enabled = true;
        sources(&cfg, d.path(), &mut s).unwrap(); // next run start: the stale watermark counts again
        assert_eq!(s.horizon().unwrap(), Some(5));
        // a git setup error leaves git out of the sources but its watermark still counts
        s.conn.execute("DROP TABLE projects", []).unwrap();
        cfg.presence.enabled = false;
        assert_eq!(sources(&cfg, d.path(), &mut s).unwrap().len(), 0);
        assert_eq!(s.horizon().unwrap(), Some(100));
    }
    #[test]
    fn a_zero_interval_ticks_every_second() {
        assert_eq!((effective_interval(0), effective_interval(15)), (1, 15));
        assert_eq!(effective_interval(u64::MAX), 86_400);
        let _ = Instant::now() + Duration::from_secs(effective_interval(u64::MAX)); // no overflow panic
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
    #[test]
    fn run_loop_returns_when_stopped() {
        let stop = std::sync::atomic::AtomicBool::new(true);
        let d = tempfile::tempdir().unwrap();
        let mut s = Store::open_in_memory().unwrap();
        let mut cfg = Config::default();
        cfg.run.interval_secs = 3600;
        cfg.claude_code.enabled = false;
        cfg.presence.enabled = false;
        run_loop(&mut s, &cfg, d.path(), &stop).unwrap();
    }
    #[test]
    fn a_disabled_publish_hook_makes_no_git_call() {
        let d = tempfile::tempdir().unwrap();
        let mut s = Store::open_in_memory().unwrap();
        let mut cfg = Config::default();
        cfg.publish.enabled = false;
        cfg.publish.remote = Some(d.path().join("nope").to_string_lossy().into_owned());
        let mut hks = hooks(&cfg, d.path());
        let r = run_once(&mut s, &mut [], &mut hks, 0);
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        assert!(!d.path().join("publish").exists());
        assert_eq!(s.meta_get("last_publish_day").unwrap(), None);
    }
    #[test]
    fn a_run_config_write_failure_is_fatal_for_sources() {
        let d = tempfile::tempdir().unwrap();
        let mut s = Store::open(&d.path().join("tally.db")).unwrap();
        s.conn.execute_batch("CREATE TRIGGER no_meta BEFORE INSERT ON meta BEGIN SELECT RAISE(ABORT, 'no'); END;").unwrap();
        assert!(sources(&Config::default(), d.path(), &mut s).is_err());
    }
}
