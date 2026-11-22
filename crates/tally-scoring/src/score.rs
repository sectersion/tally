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
    let mut sorted = events.to_vec();
    sorted.sort_by(|a, b| (a.ts, &a.id).cmp(&(b.ts, &b.id)));
    let mut seen = BTreeSet::new();
    sorted.retain(|e| seen.insert(e.id.clone()));

    let mut prev = None;
    let mut out = Vec::new();
    for (i, a) in sorted.iter().enumerate() {
        if a.is_human_action() {
            out.push(claim(&sorted, i, prev));
            prev = Some(a.ts);
        }
    }
    out
}

/// `ev` sorted by `(ts, id)`; `t_prev` is the previous human action's ts, if any.
fn claim(ev: &[Event], i: usize, t_prev: Option<i64>) -> Claim {
    let a = &ev[i];
    let t = a.ts;
    let iv = Span { start: t_prev.unwrap_or(t - FIRST_ACTION_MS).max(t - MAX_INTERVAL_MS), end: t };

    let (mut reading, mut idle_part, mut agent_idle, mut busy) = (vec![], vec![], vec![], vec![]);
    // ponytail: scans every event per action, O(n^2); index spans by ts if runs get long.
    for e in ev {
        match e.kind {
            Kind::AgentOutput { chars } if iv.start <= e.ts && e.ts <= t => {
                let len = (chars as i64 * 1000 / READ_CPS).min(MAX_READ_MS);
                reading.push(Span { start: e.ts, end: e.ts + len });
            }
            // Grace is measured from the idle span's real start, before clipping to the interval.
            Kind::Idle { end_ts } => {
                idle_part.push(Span { start: e.ts + IDLE_GRACE_MS, end: end_ts });
                agent_idle.push(Span { start: e.ts + AGENT_IDLE_MS, end: end_ts });
            }
            Kind::AgentBusy { end_ts } => busy.push(Span { start: e.ts, end: end_ts }),
            _ => {}
        }
    }
    let busy_part = subtract(&busy, &subtract(&busy, &agent_idle)); // busy ∩ agent_idle
    let think = match a.kind {
        Kind::Prompt { relevance_pm, .. } if relevance_pm >= THINK_RELEVANCE_PM => {
            vec![Span { start: t - THINK_MAX_MS, end: t }]
        }
        _ => vec![],
    };

    let all = [idle_part.as_slice(), &busy_part].concat();
    let prot = [reading.as_slice(), &think].concat();
    let removed = |src: &[Span], prot: &[Span]| total_ms(&clip(&subtract(src, prot), iv));
    let removed_ms = removed(&all, &prot);

    let ctx = CapContext {
        interrupt_followed_by_prompt: matches!(a.kind, Kind::Interrupt)
            && ev[i + 1..]
                .iter()
                .take_while(|e| e.ts - t <= INTERRUPT_FOLLOW_MS)
                .any(|e| matches!(e.kind, Kind::Prompt { .. })),
        completes_fail_pass_loop: completes_fail_pass_loop(ev, i),
    };
    let raw_ms = iv.end - iv.start - removed_ms;
    let cap = cap_ms(a, &ctx);
    let verified_ms = raw_ms.min(cap);

    let reasons = [
        ("first_action", t_prev.is_none()),
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
        unverified_s: ((raw_ms - verified_ms) / 1000) as u32,
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
