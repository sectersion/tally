//! Prints the server test fixture: 16 six-entry chains, one per project `acme/demo-N`.
//! Regenerate with `cd server && bun run fixture`.
use std::collections::BTreeMap;

use tally_scoring::event::{Event, Kind};
use tally_scoring::ledger::{chain, GENESIS_HASH};
use tally_scoring::score;

fn main() {
    let mut out = BTreeMap::new();
    for n in 1..=16 {
        let name = format!("acme/demo-{n}");
        let events: Vec<Event> = (0..6)
            .map(|i| Event {
                v: 1,
                id: (i + 1).to_string(),
                ts: i * 600 * 1000,
                source: "t".into(),
                project: Some(name.clone()),
                session: None,
                kind: Kind::Prompt { chars: 200, relevance_pm: 600, novelty_bytes: 90 },
            })
            .collect();
        out.insert(name, chain(&score(&events), GENESIS_HASH));
    }
    println!("{}", serde_json::to_string_pretty(&out).unwrap());
}
