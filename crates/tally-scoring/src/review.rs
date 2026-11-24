//! Labelled scoring and reviewer adjustments (spec section 3.2, steps 9 to 11).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::event::{Event, Kind};
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
}
