//! Runs scenarios through `tally_scoring::score` and reports credit per attack.
use crate::scenarios::{all, control, Layer, Scenario, NOT_MODELLED};
use tally_scoring::{score, RULESET_VERSION};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub name: &'static str,
    pub duration_s: i64,
    pub verified_s: i64,
    pub unverified_s: i64,
    pub reasons: Vec<String>,
}

pub fn run(s: &Scenario) -> Outcome {
    let claims = score(&s.events);
    let mut reasons: Vec<String> = claims.iter().flat_map(|c| c.reasons.clone()).collect();
    reasons.sort();
    reasons.dedup();
    Outcome {
        name: s.name,
        duration_s: s.duration_s,
        verified_s: claims.iter().map(|c| i64::from(c.verified_s)).sum(),
        unverified_s: claims.iter().map(|c| i64::from(c.unverified_s)).sum(),
        reasons,
    }
}

fn layer(l: Layer) -> &'static str {
    match l {
        Layer::Scorer => "scorer",
        Layer::Adapter => "adapter",
        Layer::Judge => "judge",
    }
}

/// Attacks credited more than this share of their duration are listed under Gaps.
const GAP_PCT: i64 = 5;

pub fn render(scoring_commit: &str) -> String {
    let ctl = run(&control());
    let rows: Vec<(Scenario, Outcome)> = all().into_iter().map(|s| { let o = run(&s); (s, o) }).collect();
    let mut md = format!(
        "# Red-team report\n\nRuleset `{RULESET_VERSION}`, `tally-scoring` at commit `{scoring_commit}`.\n\nThe harness drives the scorer with the events each adapter would emit. It does not run real harnesses, so the assumption column says what the adapter or judge must have done for these events to be the right input.\n\nHonest control: {} min credited out of {} min.\n\n| attack | relies on | simulated minutes | credited minutes | credited as % of honest control | assumption |\n|---|---|---|---|---|---|\n",
        ctl.verified_s / 60,
        ctl.duration_s / 60
    );
    for (s, o) in &rows {
        let pct = if ctl.verified_s > 0 { o.verified_s * 100 / ctl.verified_s } else { 0 };
        md.push_str(&format!("| `{}` | {} | {} | {} | {}% | {} |\n", s.name, layer(s.relies_on), o.duration_s / 60, o.verified_s / 60, pct, s.assumption));
    }
    md.push_str("\n## Not modelled here\n\n");
    for (n, why) in NOT_MODELLED {
        md.push_str(&format!("- `{n}`: {why}\n"));
    }
    md.push_str("\n## Per attack\n\n");
    for (s, o) in &rows {
        md.push_str(&format!("### `{}`\n\nPitch row: {}.\n\nCredited {} s verified, {} s unverified. Reasons: {}.\n\n", s.name, s.pitch_row, o.verified_s, o.unverified_s, if o.reasons.is_empty() { "none".into() } else { o.reasons.join(", ") }));
    }
    md.push_str("## Gaps\n\n");
    let gaps: Vec<_> = rows.iter().filter(|(_, o)| o.verified_s * 100 > o.duration_s * GAP_PCT).collect();
    if gaps.is_empty() {
        md.push_str("No attack was credited more than 5 percent of its duration.\n");
    }
    for (s, o) in gaps {
        md.push_str(&format!("- `{}`: credited {} of {} minutes ({}%), relies on the {}. {}\n", s.name, o.verified_s / 60, o.duration_s / 60, o.verified_s * 100 / o.duration_s, layer(s.relies_on), if s.name == "planted_failing_test" { "KNOWN GAP: spec section 8 defers the 'failing test existed before the loop' check, so this earns credit today." } else { "" }));
    }
    md
}
