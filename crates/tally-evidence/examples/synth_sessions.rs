//! Writes three SYNTHETIC demo sessions and their replay reports into the directory given as the first argument.
//! cargo run -p tally-evidence --example synth_sessions -- docs/evidence/replay
use std::path::Path;
use tally_evidence::build::*;
use tally_evidence::replay::{compare, render_markdown};
use tally_scoring::Event;

const MIN: i64 = 60_000;

fn agent_away() -> Vec<Event> {
    vec![prompt("p1", 0, 200, 400, 100), busy("b", 0, 60 * MIN), idle("i", 5 * MIN, 55 * MIN), prompt("p2", 58 * MIN, 200, 400, 100)]
}

/// Five projects, one prompt every 10 minutes (a thoughtful prompt each time): sparse heartbeats that a
/// timeout-based tool undercounts, while the human really was working the whole afternoon.
fn five_project_afternoon() -> Vec<Event> {
    (0..18).map(|i| prompt(&format!("p{i}"), i * 10 * MIN, 300, 700, 150)).collect()
}

fn steady() -> Vec<Event> {
    (0..30).map(|i| if i % 2 == 0 { prompt(&format!("p{i}"), i * MIN, 200, 700, 100) } else { edit(&format!("e{i}"), i * MIN, 5) }).collect()
}

fn main() {
    let dir = std::env::args().nth(1).expect("usage: synth_sessions <out dir>");
    let dir = Path::new(&dir);
    std::fs::create_dir_all(dir).unwrap();
    for (name, title, ev) in [
        ("agent-away", "Synthetic: the agent runs while the user is away", agent_away()),
        ("five-project-afternoon", "Synthetic: a prompt every 10 minutes across five projects", five_project_afternoon()),
        ("steady-work", "Synthetic: steady human work with no agent", steady()),
    ] {
        let nd: String = ev.iter().map(|e| serde_json::to_string(e).unwrap() + "\n").collect();
        std::fs::write(dir.join(format!("{name}.ndjson")), nd).unwrap();
        std::fs::write(dir.join(format!("{name}.md")), render_markdown(&compare(&ev, None), title)).unwrap();
        if name == "steady-work" {
            // a ledger of the synthetic session, the input of `tally-evidence cost` until a real published ledger exists
            let led = tally_scoring::chain(&tally_scoring::score(&ev), tally_scoring::GENESIS_HASH);
            std::fs::write(dir.join("steady-work.ledger.json"), serde_json::to_string(&led).unwrap()).unwrap();
        }
    }
}
