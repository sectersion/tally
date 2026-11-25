//! Labelled scoring and reviewer adjustments (spec section 3.2, steps 9 to 11).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::event::{Event, Kind};
use crate::ruleset::RULESET_VERSION;
use crate::score::{score, sort_dedup, Claim};

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Label {
    Steering,
    Testing,
    Debugging,
    Repetitive,
    DarkFlow,
}

impl Label {
    /// Whether the labelled turn's verified seconds become unverified.
    pub fn forfeits(self) -> bool {
        matches!(self, Label::Repetitive | Label::DarkFlow)
    }
    pub fn reason(self) -> &'static str {
        match self {
            Label::Steering => "label_steering",
            Label::Testing => "label_testing",
            Label::Debugging => "label_debugging",
            Label::Repetitive => "label_repetitive",
            Label::DarkFlow => "label_dark_flow",
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Adjustment {
    pub action_id: String,
    #[serde(default)]
    pub approve_unverified_s: u32,
    #[serde(default)]
    pub remove_verified_s: u32,
    pub reviewer: String,
    pub reason: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct Review {
    #[serde(default)]
    pub labels: BTreeMap<String, Label>,
    #[serde(default)]
    pub adjustments: Vec<Adjustment>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Reviewed {
    pub claims: Vec<Claim>,
    /// Label keys and adjustment ids that matched no claim (or a non-prompt claim, for labels). Sorted, deduped.
    pub unknown_ids: Vec<String>,
}

fn push_reason(c: &mut Claim, r: &str) {
    if !c.reasons.iter().any(|x| x == r) {
        c.reasons.push(r.to_string());
    }
}

/// `score(events)` with the review's labels (steps 9, 10) and adjustments (step 11) applied.
pub fn score_with_labels(events: &[Event], review: &Review) -> Reviewed {
    let mut claims = score(events);
    let mut unknown: BTreeSet<String> = BTreeSet::new();

    // Turn of each claim: (own prompt?, label of its turn). Claims are in the order of human actions.
    let mut current: BTreeMap<&str, Option<Label>> = BTreeMap::new();
    let sorted = sort_dedup(events);
    let humans = sorted.iter().filter(|e| e.is_human_action());
    for (c, e) in claims.iter_mut().zip(humans) {
        let (is_prompt, label) = if matches!(e.kind, Kind::Prompt { .. }) {
            let l = review.labels.get(&e.id).copied();
            if let Some(s) = e.session.as_deref() {
                current.insert(s, l);
            }
            (true, l)
        } else {
            (false, e.session.as_deref().and_then(|s| current.get(s).copied().flatten()))
        };
        let Some(l) = label else { continue };
        if l.forfeits() {
            c.unverified_s += c.verified_s;
            c.verified_s = 0;
            push_reason(c, l.reason());
        } else if is_prompt {
            push_reason(c, l.reason());
        }
    }

    let by_id: BTreeMap<&str, usize> = claims.iter().enumerate().map(|(i, c)| (c.action_id.as_str(), i)).collect();
    for id in review.labels.keys() {
        if by_id.get(id.as_str()).is_none_or(|&i| claims[i].kind != "prompt") {
            unknown.insert(id.clone());
        }
    }
    let by_id: BTreeMap<String, usize> = by_id.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
    for a in &review.adjustments {
        let Some(&i) = by_id.get(&a.action_id) else {
            unknown.insert(a.action_id.clone());
            continue;
        };
        let c = &mut claims[i];
        let moved = a.approve_unverified_s.min(c.unverified_s);
        if moved > 0 {
            c.verified_s += moved;
            c.unverified_s -= moved;
            push_reason(c, "reviewer_approved");
        }
        let moved = a.remove_verified_s.min(c.verified_s);
        if moved > 0 {
            c.verified_s -= moved;
            c.unverified_s += moved;
            push_reason(c, "reviewer_removed");
        }
    }
    Reviewed { claims, unknown_ids: unknown.into_iter().collect() }
}

/// The re-derivation file: events plus the review, for one project and time range.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RederiveInput {
    pub ruleset: String,
    pub project: String,
    pub from_ms: i64,
    pub to_ms: i64,
    pub events: Vec<Event>,
    #[serde(default)]
    pub excluded_action_ids: Vec<String>,
    pub review: Review,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct RederiveOutput {
    pub ruleset: String,
    pub claims: Vec<Claim>,
    pub verified_s: u64,
    pub unverified_s: u64,
    pub unknown_ids: Vec<String>,
}

/// Re-score `input.events` with the review and keep the project's claims in range, minus the excluded ones.
pub fn rederive(input: &RederiveInput) -> Result<RederiveOutput, String> {
    if input.ruleset != RULESET_VERSION {
        return Err(format!(
            "ruleset mismatch: input {}, this build {} (re-derive an older ruleset with the tagged build of that ruleset)",
            input.ruleset, RULESET_VERSION
        ));
    }
    let Reviewed { claims, unknown_ids } = score_with_labels(&input.events, &input.review);
    let mut unknown: BTreeSet<String> = unknown_ids.into_iter().collect();
    let mut kept: Vec<Claim> = claims
        .into_iter()
        .filter(|c| c.project.as_deref() == Some(input.project.as_str()) && (input.from_ms..=input.to_ms).contains(&c.ts))
        .collect();
    for id in &input.excluded_action_ids {
        if !kept.iter().any(|c| &c.action_id == id) {
            unknown.insert(id.clone());
        }
    }
    kept.retain(|c| !input.excluded_action_ids.contains(&c.action_id));
    let verified_s = kept.iter().map(|c| u64::from(c.verified_s)).sum();
    let unverified_s = kept.iter().map(|c| u64::from(c.unverified_s)).sum();
    Ok(RederiveOutput { ruleset: input.ruleset.clone(), claims: kept, verified_s, unverified_s, unknown_ids: unknown.into_iter().collect() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::score::tests::synthetic;

    #[test]
    fn empty_review_matches_score_on_synthetic() {
        let ev = synthetic(4000);
        assert_eq!(score_with_labels(&ev, &Review::default()).claims, score(&ev));
    }

    #[test]
    fn labels_never_change_totals() {
        let ev = synthetic(2000);
        let labels = ev
            .iter()
            .filter(|e| matches!(e.kind, Kind::Prompt { .. }))
            .map(|e| (e.id.clone(), Label::Repetitive))
            .collect();
        let got = score_with_labels(&ev, &Review { labels, ..Default::default() }).claims;
        let plain = score(&ev);
        assert_eq!(got.len(), plain.len());
        for (g, p) in got.iter().zip(&plain) {
            assert_eq!(g.verified_s + g.unverified_s, p.verified_s + p.unverified_s);
        }
    }

    fn mk(id: &str, s: i64, project: &str, kind: Kind) -> Event {
        Event { v: 1, id: id.into(), ts: s * 1000, source: "test".into(), project: Some(project.into()), session: Some("s".into()), kind }
    }
    fn p(id: &str, s: i64, project: &str) -> Event {
        mk(id, s, project, Kind::Prompt { chars: 200, relevance_pm: 600, novelty_bytes: 100 })
    }
    fn turn() -> Vec<Event> {
        vec![
            p("a", 0, "P"),
            mk("t", 60, "P", Kind::ToolDecision { decision: crate::event::Decision::Approve }),
            { let mut u = mk("u", 90, "P", Kind::ToolDecision { decision: crate::event::Decision::Approve }); u.session = Some("other".into()); u },
            p("b", 300, "P"),
        ]
    }
    fn input(events: Vec<Event>) -> RederiveInput {
        RederiveInput { ruleset: RULESET_VERSION.into(), project: "P".into(), from_ms: 0, to_ms: i64::MAX, events,
            excluded_action_ids: vec![], review: Review::default() }
    }

    #[test]
    fn rederive_filters_project_and_range_and_rejects_other_rulesets() {
        let mut ev = turn();
        ev.push(p("x", 1000, "Q"));
        let mut i = input(ev);
        let all = rederive(&i).unwrap();
        assert_eq!(all.claims.iter().map(|c| c.action_id.as_str()).collect::<Vec<_>>(), ["a", "t", "u", "b"]);
        assert_eq!(all.verified_s, all.claims.iter().map(|c| u64::from(c.verified_s)).sum::<u64>());
        i.from_ms = 60_000;
        i.to_ms = 60_000;
        assert_eq!(rederive(&i).unwrap().claims.len(), 1);
        i.ruleset = "2026.11-0".into();
        assert!(rederive(&i).unwrap_err().starts_with("ruleset mismatch: input 2026.11-0, this build "));
    }

    #[test]
    fn excluded_ids_drop_claims_but_keep_interval_starts() {
        let mut i = input(turn());
        i.excluded_action_ids = vec!["t".into(), "zzz".into()];
        let o = rederive(&i).unwrap();
        let s: Vec<_> = o.claims.iter().map(|c| (c.action_id.as_str(), c.verified_s, c.unverified_s)).collect();
        assert_eq!(s, [("a", 120, 0), ("u", 30, 0), ("b", 210, 0)]);
        assert_eq!(o.unknown_ids, ["zzz"]);
    }
}
