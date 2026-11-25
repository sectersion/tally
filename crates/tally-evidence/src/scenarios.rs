//! Red-team scenarios: the events each attack would hand the scorer, built deterministically (fixed ids, no randomness;
//! "random" prompts are modelled by the numbers the scorer derives from them, which is all it sees).
//! The harness drives the SCORER. It does not run real harnesses, so every scenario states what the adapter or judge
//! must have done for these to be the right input.
use crate::build::*;
use tally_scoring::{Actor, Event};

const S: i64 = 1000;
const MIN: i64 = 60 * S;
pub const HOUR: i64 = 60 * MIN;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    Scorer,
    Adapter,
    Judge,
}

pub struct Scenario {
    pub name: &'static str,
    /// The PITCH.md "Threat model" row this stands for.
    pub pitch_row: &'static str,
    pub relies_on: Layer,
    pub assumption: &'static str,
    pub events: Vec<Event>,
    pub duration_s: i64,
}

/// Attacks the pitch lists that this harness cannot model, and which plan covers each.
pub const NOT_MODELLED: [(&str, &str); 2] = [
    ("hourly_bot_push", "server-side: server-witnessed time without human actions earns nothing (plan 09)"),
    ("prompt_injection_in_transcript", "judge-side: the judge only routes to humans (plan 10)"),
];

fn sc(name: &'static str, pitch_row: &'static str, relies_on: Layer, assumption: &'static str, events: Vec<Event>) -> Scenario {
    Scenario { name, pitch_row, relies_on, assumption, events, duration_s: HOUR / S }
}

pub fn all() -> Vec<Scenario> {
    vec![
        sc(
            "scripted_claude_p_loop",
            "A scripted `claude -p` or `codex exec` loop",
            Layer::Adapter,
            "the Claude Code adapter emits no human prompt for a `claude -p` run because the log entry's origin is not human",
            [vec![busy("busy", 0, HOUR)], (0..60).map(|i| agent_output(&format!("o{i}"), i * MIN, 2000)).collect()].concat(),
        ),
        sc(
            "scripted_loop_misattributed_as_human",
            "A scripted loop whose prompts are attributed to the human",
            Layer::Judge,
            "templated prompts are detected by the judge; this row reports what the scorer alone credits",
            (0..60).map(|i| prompt(&format!("p{i}"), i * MIN, 40, 0, 0)).collect(),
        ),
        sc(
            "padded_random_prompts",
            "Padded or random prompts",
            Layer::Scorer,
            "relevance and novelty are computed by the daemon from the prompt text against the session context; random text scores relevance 0",
            (0..30).map(|i| prompt(&format!("p{i}"), i * 2 * MIN, 4000, 0, 3000)).collect(),
        ),
        sc(
            "planted_failing_test",
            "Planting a failing test",
            Layer::Scorer,
            "the adapter reports the user's own test runs as initiator user; the 'failing test existed before the loop' check is deferred (spec section 8)",
            planted_failing_test(),
        ),
        sc(
            "esc_spam",
            "Esc spam",
            Layer::Scorer,
            "the adapter emits one Interrupt per Esc press",
            (0..720).map(|i| interrupt(&format!("x{i}"), i * 5 * S)).collect(),
        ),
        sc(
            "trivial_commits",
            "Trivial commits",
            Layer::Scorer,
            "the git source reports each commit by the user with its changed-line count",
            (0..40).map(|i| commit(&format!("c{i}"), i * 90 * S, Actor::User, 1)).collect(),
        ),
        sc(
            "parallel_agents",
            "Several agents in parallel",
            Layer::Scorer,
            "agent activity never produces a human event",
            (0..5)
                .flat_map(|a| std::iter::once(busy(&format!("busy{a}"), 0, HOUR)).chain((0..60).map(move |i| agent_output(&format!("o{a}-{i}"), i * MIN + a * S, 2000))))
                .collect(),
        ),
        sc(
            "idle_youtube_while_agent_runs",
            "Lunch break, phone, YouTube while the agent runs",
            Layer::Scorer,
            "the presence source reports the away time as an Idle span; relevance 400 is below the think threshold so no think allowance applies",
            vec![
                prompt("p0", 0, 200, 400, 100),
                busy("busy", 0, HOUR),
                idle("idle", 3 * MIN, 58 * MIN),
                prompt("p1", 59 * MIN, 200, 400, 100),
            ],
        ),
    ]
}

/// Twelve plant-fail-fix-pass loops with fresh command hashes, five minutes apart.
fn planted_failing_test() -> Vec<Event> {
    (0..12)
        .flat_map(|l: i64| {
            let t = l * 5 * MIN;
            let h = format!("cmd{l}");
            [
                edit(&format!("e{l}a"), t, 20),
                test_run(&format!("t{l}f"), t + MIN, &h, 1, Actor::User),
                edit(&format!("e{l}b"), t + 2 * MIN, 3),
                test_run(&format!("t{l}p"), t + 3 * MIN, &h, 0, Actor::User),
            ]
        })
        .collect()
}

/// An honest hour: a thoughtful prompt every 4 minutes, the agent works for 90 s, the user reviews and edits.
pub fn control() -> Scenario {
    let mut ev = vec![];
    for i in 0..15 {
        let t = i * 4 * MIN;
        ev.push(prompt(&format!("hp{i}"), t, 250, 700, 120));
        ev.push(busy(&format!("hb{i}"), t + 5 * S, t + 95 * S));
        ev.push(agent_output(&format!("ho{i}"), t + 90 * S, 1500));
        ev.push(edit(&format!("he{i}"), t + 3 * MIN, 8));
    }
    sc("honest_control", "an honest hour of work", Layer::Scorer, "none: this is what the attacks are measured against", ev)
}

pub fn by_name(name: &str) -> Option<Scenario> {
    all().into_iter().chain(std::iter::once(control())).find(|s| s.name == name)
}
