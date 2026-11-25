//! Replays one event stream through a Hackatime-style estimate and through Tally's scorer, minute by minute.
use crate::hackatime::{hackatime_spans, heartbeats_from_events, HACKATIME_TIMEOUT_MS};
use std::collections::BTreeSet;
use tally_scoring::{score, Event, Kind};

const MIN: i64 = 60_000;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Removed {
    pub minute_ts: i64,
    pub reason: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Report {
    pub hackatime_s: i64,
    pub tally_verified_s: i64,
    pub tally_unverified_s: i64,
    /// Minutes Hackatime counted and Tally did not credit.
    pub removed: Vec<Removed>,
    /// Minutes Tally credited and Hackatime did not (the "five-project afternoon" case).
    pub added_minutes: usize,
    /// Minutes both counted.
    pub common_minutes: usize,
    pub source_of_hackatime: &'static str,
}

/// A minute belongs to a span when the span contains the minute's midpoint (so a span costs at most 30 s of rounding per end).
fn minutes_of(spans: &[(i64, i64)]) -> BTreeSet<i64> {
    let mut out = BTreeSet::new();
    for &(a, b) in spans {
        let mut m = a.div_euclid(MIN);
        while m * MIN + MIN / 2 <= b {
            if m * MIN + MIN / 2 >= a {
                out.insert(m);
            }
            m += 1;
        }
    }
    out
}

fn overlaps(m: i64, a: i64, b: i64) -> bool {
    a < (m + 1) * MIN && b > m * MIN
}

pub fn compare(events: &[Event], heartbeats: Option<Vec<i64>>) -> Report {
    let (hb, source) = match heartbeats {
        Some(h) => (h, "real heartbeat export"),
        None => (heartbeats_from_events(events), "synthetic: every non-idle event, agent spans every 30 s, 2-minute timeout"),
    };
    let spans = hackatime_spans(&hb, HACKATIME_TIMEOUT_MS);
    let hackatime_s = spans.iter().map(|(a, b)| (b - a) / 1000).sum();
    let hack = minutes_of(&spans);

    let claims = score(events);
    // A claim carries no interval; "credited" = the last verified_s seconds ending at its ts (footnote in the report).
    let credited: Vec<(i64, i64)> = claims.iter().filter(|c| c.verified_s > 0).map(|c| (c.ts - i64::from(c.verified_s) * 1000, c.ts)).collect();
    let capped: Vec<(i64, i64)> = claims.iter().filter(|c| c.reasons.iter().any(|r| r == "capped")).map(|c| (c.ts - i64::from(c.verified_s + c.unverified_s) * 1000, c.ts)).collect();
    let tally = minutes_of(&credited);

    let span_of = |want_idle: bool| -> Vec<(i64, i64)> {
        events
            .iter()
            .filter_map(|e| match e.kind {
                Kind::Idle { end_ts } if want_idle => Some((e.ts, end_ts)),
                Kind::AgentBusy { end_ts } if !want_idle => Some((e.ts, end_ts)),
                _ => None,
            })
            .collect()
    };
    let (idles, busys) = (span_of(true), span_of(false));
    let hit = |m: i64, v: &[(i64, i64)]| v.iter().any(|&(a, b)| overlaps(m, a, b));

    let removed = hack
        .difference(&tally)
        .map(|&m| Removed {
            minute_ts: m * MIN,
            reason: if hit(m, &idles) {
                "away"
            } else if hit(m, &busys) {
                "agent_only"
            } else if hit(m, &capped) {
                "capped"
            } else {
                "no_human_action"
            },
        })
        .collect();
    Report {
        hackatime_s,
        tally_verified_s: claims.iter().map(|c| i64::from(c.verified_s)).sum(),
        tally_unverified_s: claims.iter().map(|c| i64::from(c.unverified_s)).sum(),
        removed,
        added_minutes: tally.difference(&hack).count(),
        common_minutes: hack.intersection(&tally).count(),
        source_of_hackatime: source,
    }
}

pub fn render_json(r: &Report) -> String {
    serde_json::to_string_pretty(r).expect("report serializes")
}

pub fn render_markdown(r: &Report, title: &str) -> String {
    let h = |s: i64| format!("{}h {:02}m", s / 3600, s % 3600 / 60);
    let mut by: std::collections::BTreeMap<&str, usize> = Default::default();
    for x in &r.removed {
        *by.entry(x.reason).or_default() += 1;
    }
    let mut s = format!("# {title}\n\n| | |\n|---|---|\n| Hackatime-style hours | {} |\n| Tally verified | {} |\n| Tally unverified (counted, not credited) | {} |\n| Minutes Hackatime counted that Tally removed | {} |\n| Minutes Tally credited that Hackatime did not | {} |\n\n", h(r.hackatime_s), h(r.tally_verified_s), h(r.tally_unverified_s), r.removed.len(), r.added_minutes);
    if !by.is_empty() {
        s.push_str("Removed minutes by reason:\n\n");
        for (k, n) in by {
            s.push_str(&format!("- `{k}`: {n}\n"));
        }
        s.push('\n');
    }
    s.push_str(&format!("Hackatime figure source: {}.\n\n", r.source_of_hackatime));
    s.push_str("Footnotes: the Hackatime figure is an approximation of what that tool computes, not its output. \"Credited minutes\" are the last `verified_s` seconds before each claim: claims do not carry their interval, so which minutes were credited is approximate (the totals are exact).\n");
    s
}
