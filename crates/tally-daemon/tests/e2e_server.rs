//! Runs only against a live server: set TALLY_E2E_SERVER, TALLY_E2E_PROJECT_ID, TALLY_E2E_TOKEN and
//! TALLY_E2E_PUBLIC_NAME (the server project's public name). Use a fresh project per run.
use std::collections::BTreeMap;
use tally_daemon::api::Api;
use tally_daemon::store::Store;
use tally_daemon::sync::{commit_heads, sync_all};
use tally_daemon::{credentials, finalize::finalize};
use tally_scoring::{Event, Kind, RULESET_VERSION};

#[test]
fn real_server_accepts_our_chain_and_our_commit() {
    let Ok(server) = std::env::var("TALLY_E2E_SERVER") else {
        eprintln!("skipped: TALLY_E2E_SERVER not set");
        return;
    };
    let var = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("{k} must be set with TALLY_E2E_SERVER"));
    let (id, token, name) = (var("TALLY_E2E_PROJECT_ID"), var("TALLY_E2E_TOKEN"), var("TALLY_E2E_PUBLIC_NAME"));

    let home = tempfile::tempdir().unwrap();
    let mut store = Store::open(&home.path().join("tally.db")).unwrap();
    store.register_project("p1", "/e2e", Some(&name)).unwrap();
    store.set_server_id("p1", &id).unwrap();
    credentials::save_token(home.path(), &id, &token).unwrap();
    let ev = |i: i64, kind| Event { v: 1, id: format!("e{i}"), ts: i * 100_000, source: "test".into(), project: Some("p1".into()), session: None, kind };
    let events: Vec<Event> = (0..6)
        .map(|i| if i % 2 == 0 { ev(i, Kind::Prompt { chars: 200, relevance_pm: 600, novelty_bytes: 100 }) } else { ev(i, Kind::UserEdit { file_hash: "f".into(), lines_changed: 1 }) })
        .collect();
    store.append_events(&events).unwrap();
    store.set_watermark("src", 10_000_000).unwrap();
    assert!(finalize(&mut store).unwrap().appended > 0);

    let tokens = credentials::load_tokens(home.path()).unwrap();
    let reports = sync_all(&mut store, &server, &tokens);
    assert!(!reports.is_empty());
    let local = store.published_rows(&name).unwrap();
    let local: Vec<_> = local.into_iter().filter(|r| r.entry.ruleset == RULESET_VERSION).collect();
    for (p, r) in &reports {
        let r = r.as_ref().unwrap_or_else(|e| panic!("{p}: {e}"));
        assert_eq!(r.server_count, local.len() as u64, "{p}");
    }
    let again = sync_all(&mut store, &server, &tokens);
    assert!(again.iter().all(|(_, r)| r.as_ref().unwrap().uploaded == 0));

    let commits = commit_heads(&store, &server, &tokens, &mut BTreeMap::new(), 1_000);
    assert!(!commits.is_empty() && commits.iter().all(|(_, r)| r.is_ok()), "{commits:?}");
    let head = Api::new(&server, &token).head(&id, RULESET_VERSION).unwrap();
    assert_eq!(head.head_hash, local.last().unwrap().entry.hash);
}
