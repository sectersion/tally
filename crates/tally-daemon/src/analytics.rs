//! Read-only analytics over the store for the local dashboard. Nothing here writes.
//!
//! Definitions (the page prints the same text):
//! - A **bug fixed** is one passing `test_run` that `tally_scoring::caps::completes_fail_pass_loop` accepts: a
//!   passing run whose `cmd_hash` matches an earlier failing run within `LOOP_WINDOW_MS`, with a human action in
//!   between, and no passing run of that `cmd_hash` between (scoring spec section 4). The dashboard count equals
//!   the ledger entries with reason `fail_pass_loop`, once loops whose fixing run was skipped (no presence
//!   coverage) are set aside as `skipped_fixes`. That equality holds for user-initiated fixing runs only: a run the
//!   agent started is not a human action, gets no claim and no ledger reason; its loop is `by_agent` and counted in
//!   `agent_fixes`, never in `fixed` or verified time.
//! - **Time from failure to fix**: the fixing run's `ts` minus the `ts` of the first failing run of the same
//!   `cmd_hash` after the previous passing run of it, looking back at most `LOOP_WINDOW_MS`.
//! - **Prompt after a failure**: a prompt in a project whose most recent `test_run` (any initiator) failed at most
//!   `AFTER_FAILURE_MS` earlier. **Informed**: `relevance_pm >= INFORMED_PM`. These two are display constants of the
//!   dashboard, not ruleset constants.
//!
//! Limits: a bug is a failing then passing run of the same command, so only test and build runs are seen; two
//! unrelated failures of one command count as one bug; a loop longer than one hour is not seen; relevance cannot tell
//! words from an error apart from the project's own vocabulary, so a high figure after a failure is a hint, not proof.
use crate::snapshot::Snapshot;
use crate::store::StoreError;
use serde::Serialize;
use std::collections::BTreeMap;
use tally_scoring::caps::completes_fail_pass_loop;
use tally_scoring::ruleset::{FIRST_ACTION_MS, LOOP_WINDOW_MS, MAX_INTERVAL_MS};
use tally_scoring::{Actor, Event, Kind, LedgerEntry};

pub const AFTER_FAILURE_MS: i64 = 300_000;
pub const INFORMED_PM: u16 = 300;
pub const MIN_SAMPLE: u32 = 5;

/// Event kinds that can be a human action or a test run (the only kinds that can change a loop).
pub const HUMAN_AND_RUN_KINDS: [&str; 6] = ["prompt", "interrupt", "tool_decision", "user_edit", "test_run", "commit"];
const FIXES_SHOWN: usize = 200;
const PAGE: usize = 5000;

#[derive(Serialize, Debug, PartialEq)]
pub struct BugFix {
    #[serde(skip)]
    pub id: String,
    pub fixed_ts: i64,
    pub failed_ts: i64,
    pub project: Option<String>,
    pub ms_to_fix: i64,
    pub skipped: bool,
    pub by_agent: bool,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Bugs {
    pub fixed: u32,
    pub skipped_fixes: u32,
    pub agent_fixes: u32,
    pub avg_ms_to_fix: Option<i64>,
    pub median_ms_to_fix: Option<i64>,
    pub fixes: Vec<BugFix>,
}

/// `events` sorted by `(ts, id)`, deduplicated, holding test runs and human actions from `from_ts - LOOP_WINDOW_MS`
/// to `to_ts`. Returns the fixes whose `fixed_ts` is in `[from_ts, to_ts)`, oldest first.
pub fn find_bug_fixes(events: &[Event], from_ts: i64, to_ts: i64) -> Vec<BugFix> {
    let mut out = Vec::new();
    for (i, e) in events.iter().enumerate() {
        let Kind::TestRun { cmd_hash, exit_code: 0, initiator } = &e.kind else { continue };
        if e.ts < from_ts || e.ts >= to_ts || !completes_fail_pass_loop(events, i) {
            continue;
        }
        let mut failed_ts = e.ts;
        for f in events[..i].iter().rev().take_while(|f| e.ts - f.ts <= LOOP_WINDOW_MS) {
            if let Kind::TestRun { cmd_hash: h, exit_code, .. } = &f.kind {
                if h == cmd_hash {
                    if *exit_code == 0 {
                        break;
                    }
                    failed_ts = f.ts;
                }
            }
        }
        out.push(BugFix {
            id: e.id.clone(),
            fixed_ts: e.ts,
            failed_ts,
            project: e.project.clone(),
            ms_to_fix: e.ts - failed_ts,
            skipped: false,
            by_agent: *initiator == Actor::Agent,
        });
    }
    out
}

/// Every event of `kinds` in `[from_ts, to_ts)`, sorted by `(ts, id)`. Paged one kind at a time: a multi-kind query
/// needs a sort, which rescans every remaining row on every page (quadratic on a large store).
pub(crate) fn load_events(snap: &Snapshot, kinds: &[&str], from_ts: i64, to_ts: i64) -> Result<Vec<Event>, StoreError> {
    let mut all: Vec<Event> = Vec::new();
    for kind in kinds {
        let mut after = None;
        loop {
            let page = snap.events_in(&[kind], from_ts, to_ts, after, PAGE)?;
            let n = page.len();
            after = page.last().map(|e| (e.ts, e.id.clone()));
            all.extend(page);
            if n < PAGE {
                break;
            }
        }
    }
    all.sort_by(|a, b| (a.ts, &a.id).cmp(&(b.ts, &b.id)));
    Ok(all)
}

pub fn bugs(snap: &Snapshot, from_ts: i64, to_ts: i64) -> Result<Bugs, StoreError> {
    let events = load_events(snap, &HUMAN_AND_RUN_KINDS, from_ts.saturating_sub(LOOP_WINDOW_MS), to_ts)?;
    let mut fixes = find_bug_fixes(&events, from_ts, to_ts);
    let skipped: std::collections::BTreeSet<String> = snap.skipped_in(from_ts, to_ts)?.into_iter().map(|s| s.0).collect();
    for f in fixes.iter_mut().filter(|f| !f.by_agent) {
        f.skipped = skipped.contains(&f.id);
    }
    let count = |p: &dyn Fn(&BugFix) -> bool| fixes.iter().filter(|f| p(f)).count() as u32;
    let (fixed, skipped_fixes, agent_fixes) = (count(&|f| !f.by_agent && !f.skipped), count(&|f| !f.by_agent && f.skipped), count(&|f| f.by_agent));
    let mut ms: Vec<i64> = fixes.iter().filter(|f| !f.skipped).map(|f| f.ms_to_fix).collect();
    ms.sort_unstable();
    let avg_ms_to_fix = (!ms.is_empty()).then(|| ms.iter().sum::<i64>() / ms.len() as i64);
    let median_ms_to_fix = (!ms.is_empty()).then(|| ms[(ms.len() - 1) / 2]);
    fixes.reverse();
    fixes.truncate(FIXES_SHOWN);
    Ok(Bugs { fixed, skipped_fixes, agent_fixes, avg_ms_to_fix, median_ms_to_fix, fixes })
}

#[derive(Serialize, Debug, PartialEq)]
pub struct PromptGroup {
    pub prompts: u32,
    pub informed: u32,
    pub informed_pm: Option<u32>,
    pub median_relevance_pm: Option<u32>,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct PromptsAfterFailure {
    pub after_failure: PromptGroup,
    pub other: PromptGroup,
    pub enough: bool,
    pub informed_pm_threshold: u32,
    pub window_ms: i64,
}

fn group(mut rel: Vec<u16>) -> PromptGroup {
    rel.sort_unstable();
    let prompts = rel.len() as u32;
    let informed = rel.iter().filter(|&&r| r >= INFORMED_PM).count() as u32;
    PromptGroup {
        prompts,
        informed,
        informed_pm: (prompts > 0).then(|| informed * 1000 / prompts),
        median_relevance_pm: (prompts > 0).then(|| u32::from(rel[(rel.len() - 1) / 2])),
    }
}

/// `events` sorted, deduplicated, kinds `prompt` and `test_run`, starting at `from_ts - AFTER_FAILURE_MS` or
/// earlier. Only prompts with `ts` in `[from_ts, to_ts)` count; prompts with no project are ignored.
pub fn prompts_after_failure(events: &[Event], from_ts: i64, to_ts: i64) -> PromptsAfterFailure {
    let mut last_run: BTreeMap<&str, (i64, bool)> = BTreeMap::new();
    let (mut after, mut other) = (Vec::new(), Vec::new());
    for e in events {
        let Some(project) = e.project.as_deref() else { continue };
        match &e.kind {
            Kind::TestRun { exit_code, .. } => {
                last_run.insert(project, (e.ts, *exit_code != 0));
            }
            Kind::Prompt { relevance_pm, .. } if e.ts >= from_ts && e.ts < to_ts => {
                match last_run.get(project) {
                    Some(&(t, true)) if e.ts - t <= AFTER_FAILURE_MS => after.push(*relevance_pm),
                    _ => other.push(*relevance_pm),
                }
            }
            _ => {}
        }
    }
    let (after_failure, other) = (group(after), group(other));
    PromptsAfterFailure {
        enough: after_failure.prompts >= MIN_SAMPLE,
        after_failure,
        other,
        informed_pm_threshold: u32::from(INFORMED_PM),
        window_ms: AFTER_FAILURE_MS,
    }
}

pub fn prompts_after_failure_for(snap: &Snapshot, from_ts: i64, to_ts: i64) -> Result<PromptsAfterFailure, StoreError> {
    let events = load_events(snap, &["prompt", "test_run"], from_ts.saturating_sub(AFTER_FAILURE_MS), to_ts)?;
    Ok(prompts_after_failure(&events, from_ts, to_ts))
}

#[derive(Serialize, Debug, PartialEq, Default, Clone)]
pub struct Seconds {
    pub verified_s: u64,
    pub unverified_s: u64,
    pub removed_s: u64,
    pub claims: u32,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct DayProject {
    pub day: usize,
    pub project: Option<String>,
    pub seconds: Seconds,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct RemovedReason {
    pub reason: String,
    pub claims: u32,
    pub seconds: u64,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct SkippedDay {
    pub day: usize,
    pub claims: u32,
    pub seconds: u64,
}

/// Verified and unverified are separate fields everywhere and are never added together.
/// `no_presence` (claims with no presence coverage) is in none of the totals.
#[derive(Serialize, Debug, PartialEq)]
pub struct Hours {
    pub days: Vec<DayProject>,
    pub totals: Seconds,
    pub removed_by: Vec<RemovedReason>,
    pub capped: RemovedReason,
    pub no_presence: RemovedReason,
    pub no_presence_days: Vec<SkippedDay>,
    pub protected_claims: u32,
    pub away_runs: u32,
}

/// Index `i` with `edges[i] <= ts < edges[i+1]`, `None` outside.
pub fn day_index(edges: &[i64], ts: i64) -> Option<usize> {
    if edges.len() < 2 || ts < edges[0] || ts >= edges[edges.len() - 1] {
        return None;
    }
    Some(edges.partition_point(|&e| e <= ts) - 1)
}

/// The scorer's interval for a claim at `ts` after a previous human action at `prev`.
fn interval_s(prev: Option<i64>, ts: i64) -> i64 {
    match prev {
        Some(p) if ts - p <= MAX_INTERVAL_MS => (ts - p) / 1000,
        _ => FIRST_ACTION_MS / 1000,
    }
}

/// `entries` consecutive ledger rows in the window, `skipped` the `ts` of skipped claims in the window (ascending),
/// `prev_ts` the `ts` of the human action (ledger or skipped) before the window's first one. A claim belongs to the
/// day of its `ts`, so an interval crossing midnight counts wholly toward the later day.
pub fn hours(entries: &[(u64, LedgerEntry)], skipped: &[i64], prev_ts: Option<i64>, edges: &[i64]) -> Hours {
    let mut by_day: BTreeMap<(usize, Option<String>), Seconds> = BTreeMap::new();
    let mut totals = Seconds::default();
    let mut removed_by: BTreeMap<String, (u32, u64)> = BTreeMap::new();
    let mut skipped_days: BTreeMap<usize, (u32, u64)> = BTreeMap::new();
    let mut capped = (0u32, 0u64);
    let mut no_presence = (0u32, 0u64);
    let (mut protected_claims, mut away_runs) = (0, 0);
    // merge by ts; on a tie the ledger entry comes first
    let (mut i, mut j, mut prev) = (0, 0, prev_ts);
    while i < entries.len() || j < skipped.len() {
        let take_ledger = j >= skipped.len() || (i < entries.len() && entries[i].1.ts <= skipped[j]);
        let ts = if take_ledger { entries[i].1.ts } else { skipped[j] };
        let interval = interval_s(prev, ts);
        let day = day_index(edges, ts);
        if take_ledger {
            let e = &entries[i].1;
            i += 1;
            let has = |r: &str| e.reasons.iter().any(|x| x == r);
            let removed = (interval - i64::from(e.verified_s) - i64::from(e.unverified_s)).max(0) as u64;
            if let Some(d) = day {
                let s = by_day.entry((d, e.project.clone())).or_default();
                s.verified_s += u64::from(e.verified_s);
                s.unverified_s += u64::from(e.unverified_s);
                s.removed_s += removed;
                s.claims += 1;
                totals.verified_s += u64::from(e.verified_s);
                totals.unverified_s += u64::from(e.unverified_s);
                totals.removed_s += removed;
                totals.claims += 1;
                if removed > 0 {
                    let mut why: Vec<&str> = ["agent_busy_subtracted", "idle_subtracted"].into_iter().filter(|r| has(r)).collect();
                    if why.is_empty() {
                        why.push("unexplained");
                    }
                    let row = removed_by.entry(why.join("+")).or_default();
                    row.0 += 1;
                    row.1 += removed;
                }
                if has("capped") {
                    capped.0 += 1;
                    capped.1 += u64::from(e.unverified_s);
                }
                protected_claims += u32::from(has("read_protected") || has("think_protected"));
                away_runs += u32::from(has("first_action") && prev.is_some());
            }
        } else {
            j += 1;
            if let Some(d) = day {
                no_presence.0 += 1;
                no_presence.1 += interval as u64;
                let row = skipped_days.entry(d).or_default();
                row.0 += 1;
                row.1 += interval as u64;
            }
        }
        prev = Some(ts);
    }
    let row = |reason: &str, (claims, seconds): (u32, u64)| RemovedReason { reason: reason.into(), claims, seconds };
    Hours {
        days: by_day.into_iter().map(|((day, project), seconds)| DayProject { day, project, seconds }).collect(),
        totals,
        removed_by: removed_by.into_iter().map(|(k, v)| row(&k, v)).collect(),
        capped: row("capped", capped),
        no_presence: row("no_presence_coverage", no_presence),
        no_presence_days: skipped_days.into_iter().map(|(day, (claims, seconds))| SkippedDay { day, claims, seconds }).collect(),
        protected_claims,
        away_runs,
    }
}

/// Reads only the ledger rows and skipped claims inside `[edges[0], edges.last())`.
pub fn hours_for(snap: &Snapshot, edges: &[i64]) -> Result<Hours, StoreError> {
    let (Some(&lo), Some(&hi)) = (edges.first(), edges.last()) else { return Ok(hours(&[], &[], None, edges)) };
    let (a, b) = (snap.ledger_seq_at_or_after(lo)?, snap.ledger_seq_at_or_after(hi)?);
    let entries = snap.ledger_range(a, b)?;
    let skipped: Vec<i64> = snap.skipped_in(lo, hi)?.into_iter().map(|s| s.1).collect();
    let first = entries.first().map(|e| e.1.ts).into_iter().chain(skipped.first().copied()).min();
    let prev_ts = match first {
        Some(t) => snap.last_human_ts_before(t)?,
        None => None,
    };
    Ok(hours(&entries, &skipped, prev_ts, edges))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    use crate::test_support::{ev, prompt};
    use tally_scoring::score;

    fn run(id: &str, s: i64, cmd: &str, exit: i32) -> Event {
        ev(id, s * 1000, Kind::TestRun { cmd_hash: cmd.into(), exit_code: exit, initiator: Actor::User })
    }
    fn agent_run(id: &str, s: i64, cmd: &str, exit: i32) -> Event {
        ev(id, s * 1000, Kind::TestRun { cmd_hash: cmd.into(), exit_code: exit, initiator: Actor::Agent })
    }
    fn p(id: &str, s: i64) -> Event {
        prompt(id, s * 1000, 600)
    }
    fn sorted(mut v: Vec<Event>) -> Vec<Event> {
        v.sort_by(|a, b| (a.ts, &a.id).cmp(&(b.ts, &b.id)));
        v
    }
    /// A store holding `events`, with `skipped` ids recorded as skipped claims.
    fn store(events: &[Event], skipped: &[&str]) -> Store {
        let mut s = Store::open_in_memory().unwrap();
        s.append_events(events).unwrap();
        for id in skipped {
            let ts = events.iter().find(|e| e.id == *id).unwrap().ts;
            s.conn.execute("INSERT INTO skipped_claims VALUES (?1, ?2, 'no_presence_coverage')", rusqlite::params![id, ts]).unwrap();
        }
        s
    }
    #[test]
    fn load_events_pages_many_kinds_in_order_and_is_not_quadratic() {
        let kinds = ["prompt", "interrupt", "tool_decision", "user_edit", "test_run", "commit"];
        let mut s = Store::open_in_memory().unwrap();
        let tx = s.conn.transaction().unwrap();
        let mut n = 0;
        for i in 0..400_000i64 {
            let e = match i % 3 {
                0 => prompt(&format!("e{i:07}"), i, 1),
                1 => ev(&format!("e{i:07}"), i, Kind::UserEdit { file_hash: "f".into(), lines_changed: 1 }),
                _ => ev(&format!("e{i:07}"), i, Kind::AgentOutput { chars: 1 }),
            };
            n += (i % 3 != 2) as usize;
            tx.execute("INSERT INTO events(id, ts, json) VALUES (?1, ?2, ?3)", rusqlite::params![e.id, e.ts, serde_json::to_string(&e).unwrap()]).unwrap();
        }
        tx.commit().unwrap();
        let snap = s.snapshot().unwrap();
        let t = std::time::Instant::now();
        let got = load_events(&snap, &kinds, 0, i64::MAX).unwrap();
        assert_eq!(got.len(), n);
        assert!(got.windows(2).all(|w| (w[0].ts, &w[0].id) < (w[1].ts, &w[1].id)));
        assert!(t.elapsed().as_secs_f64() < 5.5, "load_events took {:?}", t.elapsed());
    }
    fn bugs_of(events: &[Event], skipped: &[&str], from: i64, to: i64) -> Bugs {
        let mut s = store(events, skipped);
        let snap = s.snapshot().unwrap();
        bugs(&snap, from, to).unwrap()
    }
    const ALL: (i64, i64) = (0, 100_000_000);

    #[test]
    fn one_loop_is_one_bug_with_time_from_first_failure() {
        let b = bugs_of(&[run("f1", 0, "c", 1), run("f2", 100, "c", 1), p("p", 150), run("ok", 300, "c", 0)], &[], ALL.0, ALL.1);
        assert_eq!((b.fixed, b.avg_ms_to_fix, b.median_ms_to_fix), (1, Some(300_000), Some(300_000)));
        assert_eq!(b.fixes[0].failed_ts, 0);
    }
    #[test]
    fn no_human_action_between_means_no_bug() {
        let b = bugs_of(&[run("f", 0, "c", 1), run("ok", 60, "c", 0)], &[], ALL.0, ALL.1);
        assert_eq!((b.fixed, b.avg_ms_to_fix, b.median_ms_to_fix), (0, None, None));
    }
    #[test]
    fn a_second_pass_does_not_count_again() {
        let b = bugs_of(&[run("f", 0, "c", 1), p("p", 10), run("ok", 60, "c", 0), run("ok2", 90, "c", 0)], &[], ALL.0, ALL.1);
        assert_eq!(b.fixed, 1);
    }
    #[test]
    fn project_without_loops_has_none_not_zero() {
        let b = bugs_of(&[p("p", 0), run("ok", 60, "c", 0)], &[], ALL.0, ALL.1);
        assert_eq!(b, Bugs { fixed: 0, skipped_fixes: 0, agent_fixes: 0, avg_ms_to_fix: None, median_ms_to_fix: None, fixes: vec![] });
    }
    #[test]
    fn a_loop_completed_by_a_skipped_run_is_skipped_not_fixed() {
        let evs = [run("f1", 0, "c", 1), p("p1", 10), run("ok1", 100, "c", 0), run("f2", 200, "d", 1), p("p2", 210), run("ok2", 500, "d", 0)];
        let b = bugs_of(&evs, &["ok2"], ALL.0, ALL.1);
        assert_eq!((b.fixed, b.skipped_fixes, b.fixes.len()), (1, 1, 2));
        assert!(b.fixes[0].skipped && !b.fixes[1].skipped, "newest first: ok2 then ok1");
        assert_eq!((b.avg_ms_to_fix, b.median_ms_to_fix), (Some(100_000), Some(100_000)));
    }
    #[test]
    fn a_loop_completed_by_an_agent_run_is_an_agent_fix_not_fixed() {
        let evs = [run("f", 0, "c", 1), p("p", 30), agent_run("claude:s1:u1:test_run", 60, "c", 0)];
        for skipped in [&[][..], &["claude:s1:u1:test_run"][..]] {
            let b = bugs_of(&evs, skipped, ALL.0, ALL.1);
            assert_eq!((b.fixed, b.skipped_fixes, b.agent_fixes), (0, 0, 1));
            assert_eq!(b.fixes.len(), 1);
            assert!(b.fixes[0].by_agent && !b.fixes[0].skipped);
            assert_eq!(b.fixes[0].ms_to_fix, 60_000);
            assert_eq!((b.avg_ms_to_fix, b.median_ms_to_fix), (Some(60_000), Some(60_000)));
        }
    }
    #[test]
    fn a_failure_older_than_the_loop_window_is_not_a_bug() {
        let b = bugs_of(&[run("f", 0, "c", 1), p("p", 10), run("ok", 3601, "c", 0)], &[], ALL.0, ALL.1);
        assert_eq!(b.fixed, 0);
    }
    #[test]
    fn count_equals_fail_pass_loop_reasons_in_scored_claims() {
        let mut evs = Vec::new();
        for i in 0..8i64 {
            let cmd = ["a", "b", "c"][(i % 3) as usize];
            let base = i * 600;
            evs.push(run(&format!("f{i}"), base, cmd, 1));
            evs.push(p(&format!("p{i}"), base + 20));
            evs.push(run(&format!("ok{i}"), base + 100 + i, cmd, if i == 5 { 1 } else { 0 }));
        }
        evs.push(run("fa", 6000, "z", 1));
        evs.push(p("pa", 6030));
        evs.push(agent_run("claude:s1:u1:test_run", 6100, "z", 0));
        let evs = sorted(evs);
        let claims = score(&evs);
        let expected = claims.iter().filter(|c| c.reasons.iter().any(|r| r == "fail_pass_loop")).count() as u32;
        let fixes = find_bug_fixes(&evs, i64::MIN, i64::MAX);
        assert_eq!(fixes.iter().filter(|f| !f.by_agent).count() as u32, expected);
        assert_eq!(fixes.iter().filter(|f| f.by_agent).count(), 1);
        assert!(claims.iter().all(|c| c.action_id != "claude:s1:u1:test_run"));
        // skip one fixing claim (drop it from the "ledger"): fixed + skipped_fixes still equals the total
        let skip = fixes.iter().find(|f| !f.by_agent).unwrap().id.clone();
        let b = bugs_of(&evs, &[&skip], ALL.0, ALL.1);
        assert_eq!((b.fixed + b.skipped_fixes, b.skipped_fixes, b.agent_fixes), (expected, 1, 1));
        let ledger_with_reason = claims.iter().filter(|c| c.action_id != skip && c.reasons.iter().any(|r| r == "fail_pass_loop")).count() as u32;
        assert_eq!(b.fixed, ledger_with_reason);
    }
    #[test]
    fn window_filters_by_fixed_ts_but_looks_back_for_the_failure() {
        let evs = [run("f", 3000, "c", 1), p("p", 3100), run("ok", 4000, "c", 0)];
        let b = bugs_of(&evs, &[], 3_500_000, 5_000_000);
        assert_eq!((b.fixed, b.fixes[0].failed_ts), (1, 3_000_000));
        assert_eq!(bugs_of(&evs, &[], 4_500_000, 5_000_000).fixed, 0);
    }

    fn claim(seq: u64, ts_s: i64, verified: u32, unverified: u32, reasons: &[&str]) -> (u64, LedgerEntry) {
        (seq, LedgerEntry {
            v: 1, ruleset: "r".into(), action_id: format!("a{seq}"), ts: ts_s * 1000, project: Some("p1".into()), kind: "prompt".into(),
            verified_s: verified, unverified_s: unverified, reasons: reasons.iter().map(|s| s.to_string()).collect(),
            features: Default::default(), prev_hash: String::new(), hash: format!("h{seq}"),
        })
    }
    const WIDE: [i64; 2] = [0, 1_000_000_000];
    #[test]
    fn day_index_respects_edges_exactly() {
        let e = [0, 100, 200];
        assert_eq!([0, 99, 100, 199, 200, -1].map(|t| day_index(&e, t)), [Some(0), Some(0), Some(1), Some(1), None, None]);
    }
    #[test]
    fn a_claim_one_millisecond_apart_lands_on_different_days_on_a_23_hour_day() {
        let e = [0, 82_800_000, 82_800_000 + 86_400_000];
        assert_eq!((day_index(&e, 82_799_999), day_index(&e, 82_800_000)), (Some(0), Some(1)));
    }
    #[test]
    fn removed_seconds_come_from_the_gap_to_the_previous_claim() {
        let h = hours(&[claim(1, 600, 300, 0, &["idle_subtracted"])], &[], Some(0), &WIDE);
        assert_eq!(h.totals.removed_s, 300);
        assert_eq!(h.removed_by, vec![RemovedReason { reason: "idle_subtracted".into(), claims: 1, seconds: 300 }]);
    }
    #[test]
    fn first_action_uses_the_fixed_interval_and_counts_away_runs() {
        let h = hours(&[claim(1, 10, 100, 0, &["first_action"]), claim(2, 10_000, 90, 0, &["first_action"])], &[], None, &WIDE);
        assert_eq!(h.totals.removed_s, 20 + 30);
        assert_eq!(h.away_runs, 1);
    }
    #[test]
    fn capped_seconds_are_unverified_not_removed() {
        let h = hours(&[claim(1, 300, 90, 210, &["capped"])], &[], Some(0), &WIDE);
        assert_eq!((h.totals.removed_s, h.capped.seconds, h.capped.claims), (0, 210, 1));
    }
    #[test]
    fn verified_and_unverified_are_never_merged() {
        let h = hours(&[claim(1, 100, 100, 0, &[]), claim(2, 200, 0, 100, &["capped"])], &[], Some(0), &WIDE);
        assert_eq!((h.totals.verified_s, h.totals.unverified_s), (100, 100));
        assert_eq!((h.days[0].seconds.verified_s, h.days[0].seconds.unverified_s), (100, 100));
    }
    #[test]
    fn the_interval_matches_the_scorer_within_one_second_per_claim() {
        let mut evs = Vec::new();
        for i in 0..30i64 {
            let t = (i * 200 + if i >= 15 { 10_000 } else { 0 }) * 1000;
            evs.push(prompt(&format!("p{i:02}"), t, if i % 3 == 0 { 700 } else { 100 }));
            evs.push(ev(&format!("e{i:02}"), t + 90_000, Kind::UserEdit { file_hash: "f".into(), lines_changed: 3 }));
            evs.push(ev(&format!("o{i:02}"), t + 1000, Kind::AgentOutput { chars: 4000 }));
            if i % 4 == 0 {
                evs.push(ev(&format!("b{i:02}"), t + 5000, Kind::AgentBusy { end_ts: t + 80_000 }));
            }
            if i % 5 == 0 {
                evs.push(ev(&format!("i{i:02}"), t + 20_000, Kind::Idle { end_ts: t + 190_000 }));
            }
        }
        let claims = score(&evs);
        let rows: Vec<(u64, LedgerEntry)> = tally_scoring::chain(&claims, tally_scoring::GENESIS_HASH).into_iter().enumerate().map(|(i, e)| (i as u64 + 1, e)).collect();
        let h = hours(&rows, &[], None, &WIDE);
        let mut prev = None;
        let mut interval_sum = 0;
        for (_, e) in &rows {
            interval_sum += interval_s(prev, e.ts);
            prev = Some(e.ts);
        }
        let got = (h.totals.verified_s + h.totals.unverified_s + h.totals.removed_s) as i64;
        let n = rows.len() as i64;
        assert!(got >= interval_sum && got - interval_sum <= n, "{got} vs {interval_sum}");
        let unexplained = h.removed_by.iter().find(|r| r.reason == "unexplained").map_or(0, |r| r.seconds);
        assert!(unexplained <= n as u64, "{unexplained}");
    }
    #[test]
    fn a_project_with_no_claims_gives_empty_hours() {
        let h = hours(&[], &[], None, &[0, 100]);
        assert!(h.days.is_empty() && h.removed_by.is_empty());
        assert_eq!((h.totals, h.no_presence.seconds, h.no_presence.claims), (Seconds::default(), 0, 0));
    }
    #[test]
    fn a_skipped_claim_is_its_own_row_and_is_the_previous_action_of_the_next_claim() {
        let h = hours(&[claim(1, 0, 120, 0, &[]), claim(2, 360, 55, 0, &["idle_subtracted"])], &[300_000], None, &WIDE);
        // B's interval is 60 s from the skipped claim at 300 s, not 360 s from A: removed 5
        assert_eq!(h.totals.removed_s, 5);
        assert_eq!((h.no_presence.claims, h.no_presence.seconds), (1, 300));
        assert_eq!(h.totals.verified_s, 175);
        assert!(h.removed_by.iter().all(|r| r.reason != "no_presence_coverage"));
        assert_eq!(h.no_presence_days, vec![SkippedDay { day: 0, claims: 1, seconds: 300 }]);
    }
    fn file_store(ledger_ts: &[i64], skipped: &[i64]) -> Store {
        let s = Store::open_in_memory().unwrap();
        for (i, t) in ledger_ts.iter().enumerate() {
            let e = claim(i as u64 + 1, *t, 10, 0, &[]).1;
            s.conn.execute("INSERT INTO ledger(hash, entry_json) VALUES (?1, ?2)", rusqlite::params![e.hash, serde_json::to_string(&e).unwrap()]).unwrap();
        }
        for (i, t) in skipped.iter().enumerate() {
            s.conn.execute("INSERT INTO skipped_claims VALUES (?1, ?2, 'no_presence_coverage')", rusqlite::params![format!("s{i}"), t * 1000]).unwrap();
        }
        s
    }
    #[test]
    fn skipped_claims_before_the_window_still_set_prev_ts() {
        let mut s = file_store(&[1000, 1100], &[990]);
        let snap = s.snapshot().unwrap();
        // window starts at 995 s: the claim at 1000 s follows the skipped claim at 990 s (interval 10 s, not 120 s)
        let h = hours_for(&snap, &[995_000, 2_000_000]).unwrap();
        assert_eq!(h.totals.claims, 2);
        assert_eq!(h.totals.removed_s, 90); // 10 s interval: 0 removed; 100 s interval: 90 removed
    }
    #[test]
    fn hours_for_reads_only_the_window() {
        let mut s = file_store(&[100, 200, 100_000, 100_100, 200_000, 200_100], &[]);
        let snap = s.snapshot().unwrap();
        let h = hours_for(&snap, &[90_000_000, 190_000_000]).unwrap();
        assert_eq!(h.totals.claims, 2);
        // the first row follows the previous day's last action (200 s) by over an hour: 120 s interval, then 100 s
        assert_eq!(h.totals.removed_s, 110 + 90);
    }
    fn pr(id: &str, ms: i64, rel: u16) -> Event {
        prompt(id, ms, rel)
    }
    fn rn(id: &str, ms: i64, exit: i32, project: &str) -> Event {
        Event { project: Some(project.into()), ..ev(id, ms, Kind::TestRun { cmd_hash: "c".into(), exit_code: exit, initiator: Actor::User }) }
    }
    #[test]
    fn a_prompt_right_after_a_failing_run_is_in_the_after_failure_group() {
        let r = prompts_after_failure(&[rn("f", 0, 1, "p1"), pr("a", 60_000, 600)], 0, i64::MAX);
        assert_eq!(r.after_failure, PromptGroup { prompts: 1, informed: 1, informed_pm: Some(1000), median_relevance_pm: Some(600) });
        assert_eq!(r.other.prompts, 0);
    }
    #[test]
    fn a_passing_run_in_between_resets_the_group() {
        let r = prompts_after_failure(&[rn("f", 0, 1, "p1"), rn("ok", 10_000, 0, "p1"), pr("a", 20_000, 600)], 0, i64::MAX);
        assert_eq!((r.after_failure.prompts, r.other.prompts), (0, 1));
    }
    #[test]
    fn five_minutes_is_the_edge() {
        let r = prompts_after_failure(&[rn("f", 0, 1, "p1"), pr("a", 300_000, 600), pr("b", 300_001, 600)], 0, i64::MAX);
        assert_eq!((r.after_failure.prompts, r.other.prompts), (1, 1));
    }
    #[test]
    fn runs_and_prompts_in_different_projects_do_not_mix() {
        let r = prompts_after_failure(&[rn("f", 0, 1, "p2"), pr("a", 1000, 600)], 0, i64::MAX);
        assert_eq!((r.after_failure.prompts, r.other.prompts), (0, 1));
    }
    #[test]
    fn threshold_is_300_per_mille_and_zero_relevance_is_not_informed() {
        let r = prompts_after_failure(&[pr("a", 1, 0), pr("b", 2, 299), pr("c", 3, 300)], 0, i64::MAX);
        assert_eq!((r.other.informed, r.other.median_relevance_pm, r.other.informed_pm), (1, Some(299), Some(333)));
        assert_eq!((r.informed_pm_threshold, r.window_ms), (300, 300_000));
    }
    #[test]
    fn empty_groups_are_none_not_zero_and_enough_needs_five() {
        let r = prompts_after_failure(&[], 0, 10);
        assert_eq!((r.after_failure.informed_pm, r.other.informed_pm, r.enough), (None, None, false));
        let mk = |n: i64| {
            let mut v = vec![rn("f", 0, 1, "p1")];
            v.extend((1..=n).map(|i| pr(&format!("p{i}"), i * 1000, 500)));
            prompts_after_failure(&v, 0, i64::MAX).enough
        };
        assert_eq!((mk(4), mk(5)), (false, true));
    }
    #[test]
    fn window_counts_only_prompts_inside_but_uses_earlier_failures() {
        let from = 1_000_000;
        let r = prompts_after_failure(&[rn("f", from - 60_000, 1, "p1"), pr("in", from + 1000, 600), pr("out", from - 1, 600)], from, from + 10_000);
        assert_eq!((r.after_failure.prompts, r.other.prompts), (1, 0));
    }
}
