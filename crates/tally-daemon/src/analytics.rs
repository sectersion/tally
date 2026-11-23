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
use tally_scoring::ruleset::LOOP_WINDOW_MS;
use tally_scoring::{Actor, Event, Kind};

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

/// Every event of `kinds` in `[from_ts, to_ts)`, in pages (each page is bounded; the window bounds the total).
pub(crate) fn load_events(snap: &Snapshot, kinds: &[&str], from_ts: i64, to_ts: i64) -> Result<Vec<Event>, StoreError> {
    let mut all: Vec<Event> = Vec::new();
    loop {
        let after = all.last().map(|e| (e.ts, e.id.clone()));
        let page = snap.events_in(kinds, from_ts, to_ts, after, PAGE)?;
        let n = page.len();
        all.extend(page);
        if n < PAGE {
            return Ok(all);
        }
    }
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
