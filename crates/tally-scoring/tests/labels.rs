//! Labelled scoring (spec section 3.2, steps 9 to 11).

use tally_scoring::event::{Decision, Event, Kind};
use tally_scoring::{score, score_with_labels, Adjustment, Label, Review};

fn mk(id: &str, s: i64, kind: Kind) -> Event {
    Event { v: 1, id: id.into(), ts: s * 1000, source: "test".into(), project: Some("P".into()), session: None, kind }
}
fn prompt_id(id: &str, s: i64, rel: u16) -> Event {
    mk(id, s, Kind::Prompt { chars: 200, relevance_pm: rel, novelty_bytes: 100 })
}
fn approve_id(id: &str, s: i64) -> Event {
    mk(id, s, Kind::ToolDecision { decision: Decision::Approve })
}
fn in_session(mut e: Event, s: &str) -> Event {
    e.session = Some(s.into());
    e
}

fn turn() -> Vec<Event> {
    vec![
        in_session(prompt_id("a", 0, 600), "s"),
        in_session(approve_id("t", 60), "s"),
        in_session(approve_id("u", 90), "other"),
        in_session(prompt_id("b", 300, 600), "s"),
    ]
}

#[test]
fn empty_review_is_plain_score() {
    assert_eq!(score_with_labels(&turn(), &Review::default()).claims, score(&turn()));
}
#[test]
fn repetitive_turn_forfeits_to_unverified_and_keeps_total() {
    let r = Review { labels: [("a".into(), Label::Repetitive), ("b".into(), Label::Steering)].into(), ..Default::default() };
    let c = score_with_labels(&turn(), &r).claims;
    let s: Vec<_> = c.iter().map(|c| (c.action_id.as_str(), c.verified_s, c.unverified_s)).collect();
    assert_eq!(s, [("a", 0, 120), ("t", 0, 60), ("u", 30, 0), ("b", 210, 0)]);
    assert!(c[0].reasons.contains(&"label_repetitive".into()) && c[1].reasons.contains(&"label_repetitive".into()));
    assert!(c[3].reasons.contains(&"label_steering".into()) && !c[2].reasons.iter().any(|r| r.starts_with("label_")));
}
#[test]
fn adjustments_clamp_and_apply_in_order() {
    let adj = |id: &str, a, r| Adjustment { action_id: id.into(), approve_unverified_s: a, remove_verified_s: r, reviewer: "rv".into(), reason: "x".into() };
    let r = Review {
        labels: [("a".into(), Label::DarkFlow)].into(),
        adjustments: vec![adj("a", 50, 0), adj("a", 500, 0), adj("b", 0, 10)],
    };
    let c = score_with_labels(&turn(), &r).claims;
    assert_eq!((c[0].verified_s, c[0].unverified_s), (120, 0)); // 50, then the remaining 70
    assert_eq!((c[3].verified_s, c[3].unverified_s), (200, 10));
    assert!(c[3].reasons.contains(&"reviewer_removed".into()));
}
#[test]
fn unknown_and_non_prompt_ids_are_reported_not_applied() {
    let r = Review { labels: [("zzz".into(), Label::Repetitive), ("t".into(), Label::Repetitive)].into(), ..Default::default() };
    let out = score_with_labels(&turn(), &r);
    assert_eq!(out.unknown_ids, ["t", "zzz"]);
    assert_eq!(out.claims, score(&turn()));
}
#[test]
fn existing_golden_numbers_are_unchanged() {
    // lunch scenario from tests/scenarios.rs
    let idle = mk("i", 60, Kind::Idle { end_ts: 1_860_000 });
    let ev = [prompt_id("p1", 0, 600), idle, prompt_id("p2", 1900, 600)];
    let c = score_with_labels(&ev, &Review::default()).claims;
    assert_eq!(c.iter().map(|c| c.verified_s).collect::<Vec<_>>(), [120, 540]);
    assert_eq!(c, score(&ev));
}
