//! Prints the server test fixture: 16 six-entry chains, one per project `acme/demo-N`, plus two
//! divergent chains for the upload tests: `acme/demo-7-alt` (another genesis chain for
//! `acme/demo-7`), `acme/demo-4-alt` (shares entries 0..3 with `acme/demo-4`, then diverges) and
//! `acme/demo-3-2099` (demo-3's entries rehashed at ruleset `2099.01-0`) and `acme/demo-14-big`
//! (entry 2 has verified_s 3_000_000_000, rehashed).
//! Regenerate with `cd server && bun run fixture`.
use std::collections::BTreeMap;

use tally_scoring::event::{Event, Kind};
use tally_scoring::ledger::{chain, entry_hash, GENESIS_HASH};
use tally_scoring::score;

fn prompts(project: &str, secs: &[i64]) -> Vec<Event> {
    secs.iter()
        .enumerate()
        .map(|(i, s)| Event {
            v: 1,
            id: (i + 1).to_string(),
            ts: s * 1000,
            source: "t".into(),
            project: Some(project.into()),
            session: None,
            kind: Kind::Prompt { chars: 200, relevance_pm: 600, novelty_bytes: 90 },
        })
        .collect()
}

fn main() {
    let mut out = BTreeMap::new();
    for n in 1..=16 {
        let name = format!("acme/demo-{n}");
        let events = prompts(&name, &[0, 600, 1200, 1800, 2400, 3000]);
        out.insert(name, chain(&score(&events), GENESIS_HASH));
    }
    let alt7 = prompts("acme/demo-7", &[1, 601, 1201, 1801, 2401, 3001]);
    out.insert("acme/demo-7-alt".into(), chain(&score(&alt7), GENESIS_HASH));
    let base = out["acme/demo-4"][..3].to_vec();
    let claims_alt = score(&prompts("acme/demo-4", &[0, 600, 1200, 1801, 2401, 3001]))[3..].to_vec();
    let mut alt4 = base.clone();
    alt4.extend(chain(&claims_alt, &base[2].hash));
    out.insert("acme/demo-4-alt".into(), alt4);
    // demo-3's chain at a ruleset the server does not know (the upload tests' unknown_ruleset case).
    let mut next = out["acme/demo-3"].clone();
    let mut prev = GENESIS_HASH.to_string();
    for e in &mut next {
        e.ruleset = "2099.01-0".into();
        e.prev_hash = prev;
        e.hash = entry_hash(e);
        prev = e.hash.clone();
    }
    out.insert("acme/demo-3-2099".into(), next);
    // a u32-sized verified_s (beyond SQL integer), rehashed
    let mut big = out["acme/demo-14"].clone();
    big[2].verified_s = 3_000_000_000;
    let mut prev = GENESIS_HASH.to_string();
    for e in &mut big {
        e.project = Some("acme/demo-14-big".into());
        e.prev_hash = prev;
        e.hash = entry_hash(e);
        prev = e.hash.clone();
    }
    out.insert("acme/demo-14-big".into(), big);
    println!("{}", serde_json::to_string_pretty(&out).unwrap());
}
