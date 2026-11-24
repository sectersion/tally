//! Hours algorithm (spec section 3): one claim per human action.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::caps::{cap_ms, completes_fail_pass_loop, CapContext};
use crate::event::{Event, Kind};
use crate::ruleset::*;
use crate::spans::{clip, subtract, total_ms, Span};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Claim {
    pub action_id: String,
    pub ts: i64,
    pub project: Option<String>,
    pub kind: String,
    pub verified_s: u32,
    pub unverified_s: u32,
    pub reasons: Vec<String>,
    pub features: BTreeMap<String, i64>,
}

/// Claims for every human action, in `(ts, id)` order. Input order and duplicate ids do not matter.
pub fn score(events: &[Event]) -> Vec<Claim> {
    let sorted = sort_dedup(events);
    let mut prev = None;
    let mut out = Vec::new();
    // Idle/busy events that may still overlap a later interval, and the next event not yet looked at.
    let (mut open, mut next) = (Vec::new(), 0);
    // ponytail: `open` holds every idle/busy span still running at iv.start, so many long overlapping
    // spans degrade to O(n * open); fine for real sessions, an interval tree if that ever shows up.
    for (i, a) in sorted.iter().enumerate() {
        if !a.is_human_action() {
            continue;
        }
        let (iv, new_run) = interval(a.ts, prev);
        // Spans start at their ts, so one with ts >= t cannot overlap [iv.start, t).
        while next < sorted.len() && sorted[next].ts < a.ts {
            if span_end(&sorted[next]).is_some() {
                open.push(&sorted[next]);
            }
            next += 1;
        }
        // iv.start never decreases, so a span ending at or before it can never overlap again.
        open.retain(|e| span_end(e).is_some_and(|end| end > iv.start));
        let lo = sorted.partition_point(|e| e.ts < iv.start);
        let hi = sorted.partition_point(|e| e.ts <= a.ts);
        out.push(claim(&sorted, i, iv, new_run, open.iter().copied(), &sorted[lo..hi]));
        prev = Some(a.ts);
    }
    out
}

/// Sorted by `(ts, kind, id)` so equal-ts order does not hinge on the id string; first occurrence of each id kept.
fn sort_dedup(events: &[Event]) -> Vec<Event> {
    let mut sorted = events.to_vec();
    sorted.sort_by_cached_key(|e| (e.ts, kind_features(&e.kind).0, e.id.clone()));
    let mut seen = BTreeSet::new();
    sorted.retain(|e| seen.insert(e.id.clone()));
    sorted
}

/// The action's interval, and whether it starts a new run (no previous action, or a gap over MAX_INTERVAL).
fn interval(t: i64, t_prev: Option<i64>) -> (Span, bool) {
    match t_prev {
        Some(p) if t.saturating_sub(p) <= MAX_INTERVAL_MS => (Span { start: p, end: t }, false),
        _ => (Span { start: t.saturating_sub(FIRST_ACTION_MS), end: t }, true),
    }
}

fn span_end(e: &Event) -> Option<i64> {
    match e.kind {
        Kind::Idle { end_ts } | Kind::AgentBusy { end_ts } => Some(end_ts),
        _ => None,
    }
}

/// `ev` sorted by `(ts, id)`. `spans` must include every idle/busy event overlapping `iv` and
/// `outputs` every event with ts in `[iv.start, t]`; extra events are filtered out here.
fn claim<'a>(
    ev: &[Event],
    i: usize,
    iv: Span,
    new_run: bool,
    spans: impl Iterator<Item = &'a Event>,
    outputs: &[Event],
) -> Claim {
    let a = &ev[i];
    let t = a.ts;
    let overlaps = |s: &Span| s.end > iv.start && s.start < iv.end;

    let (mut reading, mut idle_part, mut agent_idle, mut busy) = (vec![], vec![], vec![], vec![]);
    for e in outputs {
        if let Kind::AgentOutput { chars } = e.kind {
            if iv.start <= e.ts && e.ts <= t {
                let len = (chars as i64 * 1000 / READ_CPS).min(MAX_READ_MS);
                reading.push(Span { start: e.ts, end: e.ts.saturating_add(len) });
            }
        }
    }
    // Spans not overlapping iv cannot change anything below: every result is clipped to iv.
    for e in spans {
        match e.kind {
            // Grace is measured from the idle span's real start, before clipping to the interval.
            Kind::Idle { end_ts } => {
                idle_part.extend(Some(Span { start: e.ts.saturating_add(IDLE_GRACE_MS), end: end_ts }).filter(overlaps));
                agent_idle.extend(Some(Span { start: e.ts.saturating_add(AGENT_IDLE_MS), end: end_ts }).filter(overlaps));
            }
            Kind::AgentBusy { end_ts } => busy.extend(Some(Span { start: e.ts, end: end_ts }).filter(overlaps)),
            _ => {}
        }
    }
    let busy_part = subtract(&busy, &subtract(&busy, &agent_idle)); // busy ∩ agent_idle
    let think = match a.kind {
        Kind::Prompt { relevance_pm, .. } if relevance_pm >= THINK_RELEVANCE_PM => {
            vec![Span { start: t.saturating_sub(THINK_MAX_MS), end: t }]
        }
        _ => vec![],
    };

    let all = [idle_part.as_slice(), &busy_part].concat();
    let prot = [reading.as_slice(), &think].concat();
    let removed = |src: &[Span], prot: &[Span]| total_ms(&clip(&subtract(src, prot), iv));
    let removed_ms = removed(&all, &prot);

    let ctx = CapContext {
        interrupt_followed_by_prompt: matches!(a.kind, Kind::Interrupt)
            && ev[ev.partition_point(|e| e.ts < t)..]
                .iter()
                .take_while(|e| e.ts.saturating_sub(t) <= INTERRUPT_FOLLOW_MS)
                .any(|e| matches!(e.kind, Kind::Prompt { .. })),
        completes_fail_pass_loop: completes_fail_pass_loop(ev, i),
    };
    let raw_ms = iv.end.saturating_sub(iv.start).saturating_sub(removed_ms);
    let cap = cap_ms(a, &ctx);
    let verified_ms = raw_ms.min(cap);

    let reasons = [
        ("first_action", new_run),
        ("read_protected", removed(&all, &think) != removed_ms),
        ("think_protected", removed(&all, &reading) != removed_ms),
        ("idle_subtracted", removed(&idle_part, &prot) > 0),
        ("agent_busy_subtracted", removed(&busy_part, &prot) > 0),
        ("fail_pass_loop", ctx.completes_fail_pass_loop),
        ("interrupt_followed", ctx.interrupt_followed_by_prompt),
        ("capped", raw_ms > cap),
    ];
    let (kind, features) = kind_features(&a.kind);
    Claim {
        action_id: a.id.clone(),
        ts: t,
        project: a.project.clone(),
        kind: kind.into(),
        verified_s: (verified_ms / 1000) as u32,
        unverified_s: ((raw_ms - verified_ms) / 1000).min(u32::MAX as i64) as u32,
        reasons: reasons.iter().filter(|r| r.1).map(|r| r.0.to_string()).collect(),
        features: features.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
    }
}

/// The event's serde tag and its claim features.
fn kind_features(k: &Kind) -> (&'static str, Vec<(&'static str, i64)>) {
    match k {
        Kind::Prompt { chars, relevance_pm, novelty_bytes } => (
            "prompt",
            vec![("chars", *chars as i64), ("relevance_pm", *relevance_pm as i64), ("novelty_bytes", *novelty_bytes as i64)],
        ),
        Kind::Interrupt => ("interrupt", vec![]),
        Kind::ToolDecision { .. } => ("tool_decision", vec![]),
        Kind::UserEdit { lines_changed, .. } => ("user_edit", vec![("lines_changed", *lines_changed as i64)]),
        Kind::TestRun { exit_code, .. } => ("test_run", vec![("exit_code", *exit_code as i64)]),
        Kind::Commit { lines_changed, .. } => ("commit", vec![("lines_changed", *lines_changed as i64)]),
        Kind::AgentBusy { .. } => ("agent_busy", vec![]),
        Kind::Idle { .. } => ("idle", vec![]),
        Kind::AgentOutput { .. } => ("agent_output", vec![]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::ev;

    fn at(id: &str, s: i64, kind: Kind) -> Event {
        Event { id: id.into(), ts: s * 1000, ..ev(kind) }
    }

    /// Reference: every event is a span/output candidate for every action (the old O(n^2) scan).
    fn score_reference(events: &[Event]) -> Vec<Claim> {
        let sorted = sort_dedup(events);
        let mut prev = None;
        let mut out = Vec::new();
        for (i, a) in sorted.iter().enumerate() {
            if a.is_human_action() {
                let (iv, new_run) = interval(a.ts, prev);
                out.push(claim(&sorted, i, iv, new_run, sorted.iter(), &sorted));
                prev = Some(a.ts);
            }
        }
        out
    }

    /// Deterministic synthetic history: prompts, edits, tests, interrupts, idle/busy spans (some
    /// hours long, overlapping), outputs, same-ts ties and multi-hour gaps.
    fn synthetic(n: usize) -> Vec<Event> {
        use crate::event::Actor;
        let mut x: u64 = 0x2545_f491_4f6c_dd1d;
        let mut r = move |m: u64| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x % m) as i64
        };
        let mut ts = 0;
        (0..n)
            .map(|k| {
                ts += match r(20) {
                    0 => 0,
                    1 => 3_600_000 + r(4) * 3_000_000,
                    _ => r(600_000),
                };
                let kind = match r(9) {
                    0 | 1 => Kind::Prompt { chars: r(400) as u32, relevance_pm: r(1000) as u16, novelty_bytes: 50 },
                    2 => Kind::Idle { end_ts: ts + if r(5) == 0 { r(10_800_000) } else { r(900_000) } },
                    3 => Kind::AgentBusy { end_ts: ts + if r(5) == 0 { r(7_200_000) } else { r(600_000) } },
                    4 | 5 => Kind::AgentOutput { chars: r(20_000) as u32 },
                    6 => Kind::TestRun { cmd_hash: ["a", "b"][r(2) as usize].into(), exit_code: r(2) as i32, initiator: [Actor::User, Actor::Agent][r(2) as usize] },
                    7 => Kind::UserEdit { file_hash: "h".into(), lines_changed: 3 },
                    _ => Kind::Interrupt,
                };
                Event { id: format!("{k:06}"), ts, ..ev(kind) }
            })
            .collect()
    }

    #[test]
    fn optimized_matches_reference() {
        let events = synthetic(4000);
        let got = score(&events);
        assert_eq!(got, score_reference(&events));
        for r in ["first_action", "idle_subtracted", "agent_busy_subtracted", "read_protected", "think_protected", "fail_pass_loop", "capped"] {
            assert!(got.iter().any(|c| c.reasons.iter().any(|x| x == r)), "generator never hits {r}");
        }
    }

    /// `cargo test -p tally-scoring --release -- --ignored score_20k`
    #[test]
    #[ignore]
    fn score_20k_events_is_fast() {
        let events = synthetic(20_000);
        let start = std::time::Instant::now();
        let n = score(&events).len();
        let took = start.elapsed();
        println!("{n} claims in {took:?}");
        assert!(took < std::time::Duration::from_secs(1));
    }

    #[test]
    fn kind_tag_matches_serde() {
        let k = Kind::TestRun { cmd_hash: "h".into(), exit_code: 1, initiator: crate::event::Actor::User };
        let tag = serde_json::to_value(&k).unwrap()["kind"].as_str().unwrap().to_string();
        assert_eq!(kind_features(&k).0, tag);
    }
    #[test]
    fn idle_crossing_interval_edge_uses_real_start() {
        // idle starts 100 s before the interval; grace ends at 80 s, so 80..300 is removed.
        let c = score(&[at("a", 0, Kind::Interrupt), at("i", -100, Kind::Idle { end_ts: 300_000 }), at("b", 300, Kind::UserEdit { file_hash: "h".into(), lines_changed: 1 })]);
        assert_eq!(c[1].verified_s + c[1].unverified_s, 80);
        assert_eq!(c[1].reasons, vec!["idle_subtracted"]);
    }
    #[test]
    fn interrupt_followed_by_prompt() {
        let p = Kind::Prompt { chars: 200, relevance_pm: 600, novelty_bytes: 50 };
        let c = score(&[at("a", 0, Kind::Interrupt), at("b", 200, Kind::Interrupt), at("c", 320, p)]);
        assert_eq!(c[0].reasons, vec!["first_action", "capped"]);
        assert_eq!((c[1].verified_s, c[1].unverified_s, c[1].reasons.clone()), (120, 80, vec!["interrupt_followed".into(), "capped".into()]));
        assert_eq!(c[2].features["relevance_pm"], 600);
    }
}
