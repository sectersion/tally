//! Golden scenarios for `score` (spec section 3). Times in helpers are seconds.

use std::sync::atomic::{AtomicU32, Ordering};
use tally_scoring::event::{Actor, Event, Kind};
use tally_scoring::score;

static NEXT_ID: AtomicU32 = AtomicU32::new(0);

fn mk(s: i64, project: Option<&str>, kind: Kind) -> Event {
    Event {
        v: 1,
        id: format!("{:06}", NEXT_ID.fetch_add(1, Ordering::SeqCst)),
        ts: s * 1000,
        source: "test".into(),
        project: project.map(Into::into),
        session: None,
        kind,
    }
}
fn prompt(chars: u32, rel: u16) -> Kind {
    Kind::Prompt { chars, relevance_pm: rel, novelty_bytes: 100 }
}
fn prompt_at(s: i64, rel: u16) -> Event {
    mk(s, Some("P"), prompt(200, rel))
}
fn prompt_in(s: i64, p: &str, rel: u16) -> Event {
    mk(s, Some(p), prompt(200, rel))
}
fn prompt_chars_at(s: i64, chars: u32, rel: u16) -> Event {
    mk(s, Some("P"), prompt(chars, rel))
}
fn prompt_no_project_at(s: i64, rel: u16) -> Event {
    mk(s, None, prompt(200, rel))
}
fn edit_in(s: i64, p: &str) -> Event {
    mk(s, Some(p), Kind::UserEdit { file_hash: "h".into(), lines_changed: 3 })
}
fn edit_at(s: i64) -> Event {
    edit_in(s, "P")
}
fn test_at(s: i64, cmd: &str, exit_code: i32) -> Event {
    mk(s, Some("P"), Kind::TestRun { cmd_hash: cmd.into(), exit_code, initiator: Actor::User })
}
fn idle(a: i64, b: i64) -> Event {
    mk(a, None, Kind::Idle { end_ts: b * 1000 })
}
fn busy(a: i64, b: i64) -> Event {
    mk(a, None, Kind::AgentBusy { end_ts: b * 1000 })
}
fn output(s: i64, chars: u32) -> Event {
    mk(s, None, Kind::AgentOutput { chars })
}

#[test]
fn lunch_break_earns_grace_plus_thinking_only() {
    // prompt rel 600 at 0; idle 60..1860; prompt rel 600 at 1900
    let c = score(&[prompt_at(0, 600), idle(60, 1860), prompt_at(1900, 600)]);
    assert_eq!((c[0].verified_s, c[0].unverified_s), (120, 0));
    assert_eq!((c[1].verified_s, c[1].unverified_s), (540, 0));
    assert!(c[1].reasons.contains(&"think_protected".into()));
}
#[test]
fn watching_generation_earns_nothing_reading_does() {
    // prompt at 0; agent busy 5..305; idle 10..300; output 2500 chars at 305; prompt rel 300 at 420
    let c = score(&[prompt_at(0, 600), busy(5, 305), idle(10, 300), output(305, 2500), prompt_at(420, 300)]);
    assert_eq!(c[1].verified_s, 175);
    assert!(c[1].reasons.contains(&"agent_busy_subtracted".into()));
}
#[test]
fn reading_output_protects_idle_time_and_says_so() {
    // idle 10..300 would remove 190..300 (110 s); reading 100..200 protects 190..200, so only 100 s go
    let c = score(&[prompt_at(0, 600), idle(10, 300), output(100, 2500), prompt_at(420, 300)]);
    assert_eq!(c[1].verified_s, 320);
    assert!(c[1].reasons.contains(&"read_protected".into()));
    assert!(c[1].reasons.contains(&"idle_subtracted".into()));
}
#[test]
fn extreme_timestamps_do_not_panic() {
    let mut lo = prompt_at(0, 600);
    lo.ts = i64::MIN;
    let mut span = idle(0, 0);
    span.ts = i64::MIN;
    span.kind = Kind::Idle { end_ts: i64::MAX };
    let mut hi = prompt_at(0, 600);
    hi.ts = i64::MAX;
    let c = score(&[lo, span, hi]);
    assert_eq!(c.len(), 2);
}
#[test]
fn multitasking_splits_by_next_action() {
    let c = score(&[prompt_in(0, "A", 600), prompt_in(600, "B", 400), edit_in(700, "A")]);
    assert_eq!((c[1].project.as_deref(), c[1].verified_s, c[1].unverified_s), (Some("B"), 552, 48));
    assert_eq!((c[2].project.as_deref(), c[2].verified_s), (Some("A"), 100));
}
#[test]
fn try_again_is_capped() {
    let c = score(&[prompt_at(0, 600), prompt_chars_at(300, 9, 600)]);
    assert_eq!((c[1].verified_s, c[1].unverified_s), (90, 210));
}
#[test]
fn fail_pass_loop_gets_large_cap() {
    let c = score(&[test_at(0, "x", 1), prompt_at(300, 600), test_at(900, "x", 0)]);
    assert_eq!([c[0].verified_s, c[1].verified_s, c[2].verified_s], [120, 300, 600]);
    assert!(c[2].reasons.contains(&"fail_pass_loop".into()));
}
// Review Focus 1, 2, 5
#[test]
fn order_and_duplicates_do_not_matter() {
    let a = [prompt_at(0, 600), idle(60, 1860), prompt_at(1900, 600)];
    let mut b = vec![a[2].clone(), a[0].clone(), a[1].clone(), a[0].clone()];
    assert_eq!(score(&a), score(&b));
    b.reverse();
    assert_eq!(score(&a), score(&b));
}
#[test]
fn same_timestamp_actions_get_zero() {
    let c = score(&[prompt_at(100, 600), edit_at(100)]);
    assert_eq!(c[1].verified_s + c[1].unverified_s, 0);
}
#[test]
fn null_project_is_kept_unattributed() {
    let c = score(&[prompt_no_project_at(0, 600)]);
    assert_eq!((c[0].project.clone(), c[0].verified_s), (None, 120));
}
#[test]
fn long_gap_starts_a_new_run() {
    let c = score(&[prompt_at(0, 600), prompt_at(36_000, 600)]);
    assert_eq!((c[1].verified_s, c[1].unverified_s), (120, 0));
    assert!(c[1].reasons.contains(&"first_action".into()));
}
#[test]
fn gap_of_exactly_max_interval_is_same_run() {
    let c = score(&[prompt_at(0, 600), prompt_at(3600, 600)]);
    assert_eq!((c[1].verified_s, c[1].unverified_s), (768, 2832));
    assert!(!c[1].reasons.contains(&"first_action".into()));
}
#[test]
fn equal_ts_actions_do_not_depend_on_id_or_input_order() {
    let at = |id: &str, k: Kind| Event { id: id.into(), ..mk(600, Some("P"), k) };
    let run = |ii: &str, pi: &str, rev: bool| {
        let mut v = vec![mk(0, Some("P"), Kind::Interrupt), at(ii, Kind::Interrupt), at(pi, prompt(200, 1000))];
        if rev {
            v.reverse();
        }
        score(&v).iter().map(|c| (c.kind.clone(), c.verified_s)).collect::<Vec<_>>()
    };
    let want = run("a", "b", false);
    assert_eq!(want, run("b", "a", false));
    assert_eq!(want, run("b", "a", true));
    // same id and ts, different kind (SCORING-4)
    let x = |k| Event { id: "x".into(), ..mk(5, Some("P"), k) };
    let (a, b) = (x(prompt(200, 600)), x(Kind::Interrupt));
    assert_eq!(score(&[a.clone(), b.clone()]), score(&[b, a]));
}
#[test]
fn fail_pass_loop_completes_once() {
    let c = score(&[test_at(0, "x", 1), prompt_at(300, 600), test_at(900, "x", 0), test_at(1800, "x", 0)]);
    assert_eq!(c[2].verified_s, 600);
    assert!(c[2].reasons.contains(&"fail_pass_loop".into()));
    assert_eq!((c[3].verified_s, c[3].unverified_s), (120, 780));
    assert!(!c[3].reasons.contains(&"fail_pass_loop".into()));
}

#[test]
fn interrupt_followed_by_same_ts_prompt_regardless_of_id() {
    for pid in ["a", "z"] {
        let mut p = prompt_at(1000, 600);
        p.id = pid.into();
        let mut i = mk(1000, Some("P"), Kind::Interrupt);
        i.id = "m".into();
        let c = score(&[i, p]);
        let c = c.iter().find(|c| c.action_id == "m").unwrap();
        assert!(c.reasons.contains(&"interrupt_followed".into()), "prompt id {pid}");
    }
}
