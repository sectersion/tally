//! Per-action time caps (spec section 4) and fail->pass loop detection.

use crate::event::{Decision, Event, Kind};
use crate::ruleset::*;

const TRIVIAL_PROMPT_CAP_MS: i64 = 90_000;
const PROMPT_BASE_S: i64 = 120;
const PROMPT_MAX_MS: i64 = 1_200_000;
const INTERRUPT_FOLLOWED_CAP_MS: i64 = 120_000;
const INTERRUPT_CAP_MS: i64 = 30_000;
const APPROVE_CAP_MS: i64 = 60_000;
const DENY_CAP_MS: i64 = 180_000;
const USER_EDIT_CAP_MS: i64 = 180_000;
const TEST_RUN_CAP_MS: i64 = 120_000;
const TEST_LOOP_CAP_MS: i64 = 1_200_000;
const SMALL_COMMIT_LINES: u32 = 3;
const SMALL_COMMIT_CAP_MS: i64 = 60_000;
const COMMIT_CAP_MS: i64 = 600_000;

pub struct CapContext {
    pub interrupt_followed_by_prompt: bool,
    pub completes_fail_pass_loop: bool,
}

pub fn is_trivial_prompt(chars: u32, relevance_pm: u16) -> bool {
    chars < TRIVIAL_CHARS || relevance_pm < TRIVIAL_RELEVANCE_PM
}

pub fn cap_ms(action: &Event, ctx: &CapContext) -> i64 {
    if !action.is_human_action() {
        return 0;
    }
    match &action.kind {
        Kind::Prompt { chars, relevance_pm, .. } => {
            if is_trivial_prompt(*chars, *relevance_pm) {
                TRIVIAL_PROMPT_CAP_MS
            } else {
                ((PROMPT_BASE_S + *relevance_pm as i64 * 108 / 100) * 1000).min(PROMPT_MAX_MS)
            }
        }
        Kind::Interrupt if ctx.interrupt_followed_by_prompt => INTERRUPT_FOLLOWED_CAP_MS,
        Kind::Interrupt => INTERRUPT_CAP_MS,
        Kind::ToolDecision { decision: Decision::Approve } => APPROVE_CAP_MS,
        Kind::ToolDecision { decision: Decision::Deny } => DENY_CAP_MS,
        Kind::UserEdit { .. } => USER_EDIT_CAP_MS,
        Kind::TestRun { .. } if ctx.completes_fail_pass_loop => TEST_LOOP_CAP_MS,
        Kind::TestRun { .. } => TEST_RUN_CAP_MS,
        Kind::Commit { lines_changed, .. } if *lines_changed < SMALL_COMMIT_LINES => SMALL_COMMIT_CAP_MS,
        Kind::Commit { .. } => COMMIT_CAP_MS,
        _ => 0,
    }
}

/// `events` must be sorted by ts. Scans back for a failing run of the same command with a human
/// action strictly between; an earlier pass of that command ends the scan (it already closed the loop).
pub fn completes_fail_pass_loop(events: &[Event], idx: usize) -> bool {
    let Some(e) = events.get(idx) else { return false };
    let Kind::TestRun { cmd_hash, exit_code: 0, .. } = &e.kind else { return false };
    let mut human_between = false;
    for f in events[..idx].iter().rev().take_while(|f| e.ts - f.ts <= LOOP_WINDOW_MS) {
        if let Kind::TestRun { cmd_hash: h, exit_code, .. } = &f.kind {
            if h == cmd_hash {
                if *exit_code == 0 {
                    return false;
                }
                if human_between {
                    return true;
                }
            }
        }
        human_between |= f.is_human_action();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{ev, Actor};

    fn prompt(chars: u32, rel: u16) -> Event {
        ev(Kind::Prompt { chars, relevance_pm: rel, novelty_bytes: 0 })
    }
    fn prompt_at(ts: i64, chars: u32, rel: u16) -> Event {
        Event { ts, id: format!("p{ts}"), ..prompt(chars, rel) }
    }
    fn test_run_at(ts: i64, cmd_hash: &str, exit_code: i32) -> Event {
        Event {
            ts,
            id: format!("t{ts}"),
            ..ev(Kind::TestRun { cmd_hash: cmd_hash.into(), exit_code, initiator: Actor::User })
        }
    }
    const NONE: CapContext = CapContext { interrupt_followed_by_prompt: false, completes_fail_pass_loop: false };

    #[test]
    fn prompt_caps() {
        assert_eq!(cap_ms(&prompt(9, 900), &NONE), 90_000);
        assert_eq!(cap_ms(&prompt(200, 149), &NONE), 90_000);
        assert_eq!(cap_ms(&prompt(200, 600), &NONE), 768_000);
        assert_eq!(cap_ms(&prompt(200, 1000), &NONE), 1_200_000);
    }
    #[test]
    fn interrupt_caps() {
        assert_eq!(cap_ms(&ev(Kind::Interrupt), &CapContext { interrupt_followed_by_prompt: true, completes_fail_pass_loop: false }), 120_000);
        assert_eq!(cap_ms(&ev(Kind::Interrupt), &NONE), 30_000);
    }
    #[test]
    fn other_caps() {
        let loop_ctx = CapContext { interrupt_followed_by_prompt: false, completes_fail_pass_loop: true };
        let tr = test_run_at(0, "x", 0);
        assert_eq!(cap_ms(&tr, &NONE), 120_000);
        assert_eq!(cap_ms(&tr, &loop_ctx), 1_200_000);
        let c = |by, n| ev(Kind::Commit { sha: "a".into(), by, lines_changed: n });
        assert_eq!(cap_ms(&c(Actor::User, 2), &NONE), 60_000);
        assert_eq!(cap_ms(&c(Actor::User, 3), &NONE), 600_000);
        assert_eq!(cap_ms(&c(Actor::Agent, 3), &NONE), 0);
        assert_eq!(cap_ms(&ev(Kind::Idle { end_ts: 1 }), &NONE), 0);
        assert_eq!(cap_ms(&ev(Kind::ToolDecision { decision: Decision::Approve }), &NONE), 60_000);
        assert_eq!(cap_ms(&ev(Kind::ToolDecision { decision: Decision::Deny }), &NONE), 180_000);
        assert_eq!(cap_ms(&ev(Kind::UserEdit { file_hash: "h".into(), lines_changed: 1 }), &NONE), 180_000);
    }
    #[test]
    fn fail_pass_loop_requires_human_action_between() {
        let fail = test_run_at(0, "x", 1);
        let pass = test_run_at(600_000, "x", 0);
        assert!(!completes_fail_pass_loop(&[fail.clone(), pass.clone()], 1));
        let p = prompt_at(300_000, 200, 600);
        assert!(completes_fail_pass_loop(&[fail, p, pass], 2));
    }
    #[test]
    fn fail_pass_loop_respects_window() {
        let v = [test_run_at(0, "x", 1), prompt_at(1_000, 200, 600), test_run_at(3_600_001, "x", 0)];
        assert!(!completes_fail_pass_loop(&v, 2));
        let v = [test_run_at(0, "x", 1), prompt_at(1_000, 200, 600), test_run_at(3_600_000, "x", 0)];
        assert!(completes_fail_pass_loop(&v, 2));
        assert!(!completes_fail_pass_loop(&v, 9));
        assert!(!completes_fail_pass_loop(&v, 0));
    }
    #[test]
    fn fail_pass_loop_completes_once() {
        let v = [test_run_at(0, "x", 1), prompt_at(300_000, 200, 600), test_run_at(900_000, "x", 0), test_run_at(1_800_000, "x", 0)];
        assert!(completes_fail_pass_loop(&v, 2));
        assert!(!completes_fail_pass_loop(&v, 3));
        // A pass of a different command does not close this one's loop.
        let v = [test_run_at(0, "x", 1), prompt_at(300_000, 200, 600), test_run_at(900_000, "y", 0), test_run_at(1_800_000, "x", 0)];
        assert!(completes_fail_pass_loop(&v, 3));
    }
}
