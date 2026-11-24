#[path = "support/fake_server.rs"]
mod fake_server;
use fake_server::*;
use tally_scoring::GENESIS_HASH;

#[test]
fn fake_server_answers_head_and_rejects_a_missing_token() {
    let f = FakeServer::start("tok");
    let get = |auth: &str| {
        raw_request(
            &f.url,
            &format!("GET /v1/ledger/srv_1/head?ruleset=2026.11-0 HTTP/1.1\r\nhost: x\r\n{auth}connection: close\r\n\r\n"),
        )
    };
    let ok = get("authorization: Bearer tok\r\n");
    assert!(ok.starts_with("HTTP/1.1 200"), "{ok}");
    assert!(ok.contains(GENESIS_HASH) && ok.contains("\"count\":0"));
    assert!(get("").starts_with("HTTP/1.1 401"));
    assert_eq!(f.requests().len(), 2);
}

use tally_daemon::api::{Api, ApiError};

#[test]
fn api_maps_every_c7_status() {
    let f = FakeServer::start("tok");
    let api = Api::new(&f.url, "tok");
    assert_eq!(api.head("srv_1", R).unwrap().head_hash, GENESIS_HASH);
    let c = chain_of(3);
    assert_eq!(api.upload("srv_1", GENESIS_HASH, &c[..2]).unwrap().count, 2);
    // A fork of entry 0 (same prev, different content) is a divergence, not a bad chain.
    let mut fork = c[..1].to_vec();
    fork[0].ts += 1;
    fork[0].hash = tally_scoring::ledger::entry_hash(&fork[0]);
    assert!(matches!(api.upload("srv_1", GENESIS_HASH, &fork), Err(ApiError::Conflict { head_hash }) if head_hash == c[1].hash));
    let mut bad = c[2..].to_vec();
    bad[0].verified_s += 1;
    assert!(matches!(api.upload("srv_1", &c[1].hash, &bad), Err(ApiError::Rejected { index: 0, .. })));
    api.commit("srv_1", &c[1].hash, 123).unwrap();
    assert_eq!(f.commits("srv_1"), [(c[1].hash.clone(), 123)]);
    f.fail_next(1, 503);
    assert!(matches!(api.head("srv_1", R), Err(e) if e.retryable()));
    f.down(true);
    assert!(matches!(api.head("srv_1", R), Err(ApiError::Network(_))));
    f.down(false);
    assert!(matches!(Api::new(&f.url, "wrong").head("srv_1", R), Err(ApiError::Unauthorized)));
    f.add_project("srv_2", "owner/other", "tok2");
    assert!(matches!(Api::new(&f.url, "tok2").head("srv_1", R), Err(ApiError::Unauthorized)));
    f.max_batch(1);
    api.upload("srv_1", &c[1].hash, &c[2..]).unwrap();
    assert!(matches!(Api::new(&f.url, "tok").upload("srv_1", GENESIS_HASH, &chain_of(2)), Err(ApiError::TooLarge)));
    assert!(matches!(api.head("srv_nope", R), Err(ApiError::Unauthorized)));
    assert_eq!(upload_sizes(&f), [2, 1, 1, 1, 2]);
    let ua = concat!("tally/", env!("CARGO_PKG_VERSION"));
    assert!(f.requests().iter().all(|r| r.headers.iter().any(|h| h.0 == "user-agent" && h.1 == ua)));
}

#[test]
fn api_treats_a_redirect_as_a_retryable_error() {
    let f = FakeServer::start("tok");
    let api = Api::new(&f.url, "tok");
    f.fail_next(1, 302);
    assert!(matches!(api.commit("srv_1", GENESIS_HASH, 1), Err(ApiError::Server(302))));
    f.fail_next(1, 302);
    let e = api.head("srv_1", R).unwrap_err();
    assert!(matches!(e, ApiError::Server(302)) && e.retryable());
}

#[test]
fn api_errors_never_contain_the_token() {
    let f = FakeServer::start("tok");
    f.down(true);
    let e = Api::new(&f.url, "tok_SECRET").head("srv_1", R).unwrap_err();
    assert!(!format!("{e} {e:?}").contains("tok_SECRET"));
}

#[test]
fn chain_helpers_are_valid_chains() {
    assert!(tally_scoring::verify_chain(&chain_of_ruleset(3, "2026.01-9"), GENESIS_HASH).is_ok());
    assert_eq!(chain_of_with_project(1, "a/b")[0].project.as_deref(), Some("a/b"));
}

// ---- sync (Task 6) ----
use std::collections::BTreeMap;
use tally_daemon::finalize::{finalize, FinalizeError};
use tally_daemon::store::Store;
use tally_daemon::sync::{sync_all, sync_chain, SyncError};
use tally_scoring::{Event, Kind};

fn api(f: &FakeServer) -> Api {
    Api::new(&f.url, "tok")
}

#[test]
fn uploads_everything_in_bounded_batches_and_a_rerun_sends_nothing() {
    let f = FakeServer::start("tok");
    let local = chain_of(1201);
    let r = sync_chain(&api(&f), "srv_1", "owner/repo", R, &local).unwrap();
    assert_eq!((r.uploaded, r.server_count), (1201, 1201));
    assert_eq!(f.chain("srv_1", R), local);
    assert_eq!(upload_sizes(&f), [500, 500, 201]);
    assert_eq!(sync_chain(&api(&f), "srv_1", "owner/repo", R, &local).unwrap().uploaded, 0);
}

#[test]
fn a_batch_stored_but_unacknowledged_is_not_resent() {
    let f = FakeServer::start("tok");
    let local = chain_of(700);
    f.store_then_drop_next_upload();
    assert!(matches!(sync_chain(&api(&f), "srv_1", "p", R, &local), Err(SyncError::Api(ApiError::Network(_)))));
    assert_eq!(f.chain("srv_1", R).len(), 500);
    let r = sync_chain(&api(&f), "srv_1", "p", R, &local).unwrap();
    assert_eq!(r.uploaded, 200);
    assert_eq!(upload_sizes(&f), [500, 200]);
    assert_eq!(f.chain("srv_1", R), local);
}

#[test]
fn a_server_head_inside_the_local_chain_resumes_after_it() {
    let f = FakeServer::start("tok");
    let local = chain_of(10);
    f.set_chain("srv_1", R, local[..7].to_vec());
    assert_eq!(sync_chain(&api(&f), "srv_1", "p", R, &local).unwrap().uploaded, 3);
}

#[test]
fn an_oversized_batch_is_halved_until_it_fits() {
    let f = FakeServer::start("tok");
    let local = chain_of(10);
    f.max_batch(3);
    assert_eq!(sync_chain(&api(&f), "srv_1", "owner/repo", R, &local).unwrap().uploaded, 10);
    assert_eq!(f.chain("srv_1", R), local);
}

#[test]
fn a_single_entry_too_large_is_returned_as_the_error() {
    let f = FakeServer::start("tok");
    f.max_batch(0);
    assert!(matches!(sync_chain(&api(&f), "srv_1", "p", R, &chain_of(2)), Err(SyncError::Api(ApiError::TooLarge))));
}

#[test]
fn a_server_head_this_device_never_wrote_stops_the_project() {
    let f = FakeServer::start("tok");
    let local = chain_of(5);
    f.set_chain("srv_1", R, chain_of_with_project(3, "someone/else"));
    assert!(matches!(sync_chain(&api(&f), "srv_1", "p", R, &local), Err(SyncError::ForeignHead { server_count: 3, .. })));
    assert_eq!(upload_sizes(&f), Vec::<usize>::new());
}

// The fake (like server/src/ledger.ts) accepts any overlap whose hashes match, so "another
// device appended a prefix" is not a 409. A 409 needs a prev_hash the server no longer has;
// this test rolls the server chain back between our GET and our POST.
#[test]
fn a_409_race_refetches_the_head_and_finishes() {
    let f = FakeServer::start("tok");
    let local = chain_of(6);
    f.set_chain("srv_1", R, local[..3].to_vec());
    let back = local[..1].to_vec();
    f.on_next_upload(move |s| {
        s.chains.insert(("srv_1".into(), R.into()), back);
    });
    let r = sync_chain(&api(&f), "srv_1", "owner/repo", R, &local).unwrap();
    assert_eq!((r.uploaded, r.server_count), (5, 6));
    assert_eq!(f.chain("srv_1", R), local);
    assert_eq!(upload_sizes(&f), [3, 5]); // the stale POST (409), then the resumed one
}

#[test]
fn a_409_that_reveals_a_foreign_head_stops() {
    let f = FakeServer::start("tok");
    let local = chain_of(6);
    f.set_chain("srv_1", R, local[..2].to_vec());
    let mut fork = local[..2].to_vec();
    fork[1].ts += 1;
    fork[1].hash = tally_scoring::ledger::entry_hash(&fork[1]);
    f.on_next_upload(move |s| {
        s.chains.insert(("srv_1".into(), R.into()), fork);
    });
    assert!(matches!(sync_chain(&api(&f), "srv_1", "p", R, &local), Err(SyncError::ForeignHead { server_count: 2, .. })));
    assert_eq!(upload_sizes(&f), [4]); // one rejected POST, none after the refetch
}

#[test]
fn a_422_is_surfaced_with_its_index_and_not_retried() {
    let f = FakeServer::start("tok");
    let mut local = chain_of(3);
    local[1].verified_s += 1; // hash no longer matches
    assert!(matches!(sync_chain(&api(&f), "srv_1", "p", R, &local), Err(SyncError::Api(ApiError::Rejected { index: 1, .. }))));
    assert_eq!(upload_sizes(&f), [3]);
}

#[test]
fn an_older_ruleset_chain_is_appended_but_an_unknown_one_is_refused() {
    let f = FakeServer::start("tok");
    let old = chain_of_ruleset(5, "old-0");
    f.set_chain("srv_1", "old-0", old[..3].to_vec());
    f.set_current_ruleset(R);
    assert_eq!(sync_chain(&api(&f), "srv_1", "owner/repo", "old-0", &old).unwrap().uploaded, 2);
    assert_eq!(f.chain("srv_1", "old-0"), old);
    let never = chain_of_ruleset(2, "older-0");
    assert!(matches!(sync_chain(&api(&f), "srv_1", "owner/repo", "older-0", &never),
        Err(SyncError::Api(ApiError::Rejected { reason, .. })) if reason == "unknown_ruleset"));
    assert_eq!(upload_sizes(&f), [2, 2]);
}

fn prompt(id: &str, ts: i64, project: &str) -> Event {
    Event {
        v: 1,
        id: id.into(),
        ts,
        source: "test".into(),
        project: Some(project.into()),
        session: None,
        kind: Kind::Prompt { chars: 200, relevance_pm: 600, novelty_bytes: 100 },
    }
}

/// Finalized, published projects `p1` = owner/repo (srv_1) and `p2` = owner/other (srv_2).
fn two_project_store(path: Option<&std::path::Path>) -> Store {
    let mut s = match path {
        Some(p) => Store::open(p).unwrap(),
        None => Store::open_in_memory().unwrap(),
    };
    s.register_project("p1", "C:/r", Some("owner/repo")).unwrap();
    s.register_project("p2", "C:/o", Some("owner/other")).unwrap();
    s.set_server_id("p1", "srv_1").unwrap();
    s.set_server_id("p2", "srv_2").unwrap();
    s.append_events(&[prompt("a", 0, "p1"), prompt("b", 100_000, "p2")]).unwrap();
    s.set_watermark("src", 1_000_000).unwrap();
    finalize(&mut s).unwrap();
    s
}

fn tokens() -> BTreeMap<String, String> {
    BTreeMap::from([("srv_1".into(), "tok".into()), ("srv_2".into(), "tok2".into())])
}

#[test]
fn a_revoked_token_stops_only_its_project() {
    let f = FakeServer::start("tok");
    f.add_project("srv_2", "owner/other", "tok2");
    f.revoke("srv_1");
    let mut s = two_project_store(None);
    let out: Vec<_> = sync_all(&mut s, &f.url, &tokens()).into_iter().map(|r| r.1).collect();
    assert_eq!(out.len(), 2);
    assert!(out.iter().any(|r| matches!(r, Err(SyncError::Api(ApiError::Unauthorized)))));
    let ok = out.iter().find_map(|r| r.as_ref().ok()).unwrap();
    assert_eq!((ok.project.as_str(), ok.uploaded), ("owner/other", 1));
    assert_eq!(f.requests().iter().filter(|r| r.path.contains("srv_1")).count(), 1);
    assert_eq!(f.chain("srv_2", R).len(), 1);
}

#[test]
fn a_linked_project_without_a_token_is_reported_and_others_continue() {
    let f = FakeServer::start("tok");
    f.add_project("srv_2", "owner/other", "tok2");
    let mut s = two_project_store(None);
    let mut t = tokens();
    t.remove("srv_2");
    let out: Vec<_> = sync_all(&mut s, &f.url, &t).into_iter().map(|r| r.1).collect();
    assert!(out.iter().any(|r| matches!(r, Err(SyncError::NotLoggedIn { project }) if project == "owner/other")));
    assert!(out.iter().any(|r| matches!(r, Ok(c) if c.project == "owner/repo" && c.uploaded == 1)));
    let again = sync_all(&mut s, &f.url, &t);
    assert!(again.iter().any(|r| matches!(&r.1, Ok(c) if c.uploaded == 0)));
}

#[test]
fn sync_all_uploads_nothing_when_the_ledger_diverges() {
    let f = FakeServer::start("tok");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.db");
    let mut s = two_project_store(Some(&path));
    // what a different build would have written: other features, correct hash
    let mut e = s.ledger().unwrap().remove(0);
    let key = e.features.keys().next().expect("prompt claims carry features").clone();
    *e.features.get_mut(&key).unwrap() += 1;
    e.hash = tally_scoring::ledger::entry_hash(&e);
    let c = rusqlite::Connection::open(&path).unwrap();
    let names: Vec<String> = c
        .prepare("SELECT name FROM sqlite_master WHERE type='trigger'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    for n in names {
        c.execute(&format!("DROP TRIGGER {n}"), []).unwrap();
    }
    c.execute(
        "UPDATE ledger SET hash = ?1, entry_json = ?2 WHERE seq = 1",
        rusqlite::params![e.hash, serde_json::to_string(&e).unwrap()],
    )
    .unwrap();
    drop(c);
    let out = sync_all(&mut s, &f.url, &tokens());
    assert!(matches!(&out[..], [(l, Err(SyncError::Derivation(FinalizeError::Divergence { .. })))] if l == "ledger"), "{out:?}");
    assert!(f.requests().is_empty());
}

#[test]
fn a_ruleset_bump_still_uploads_the_old_chains_tail() {
    let f = FakeServer::start("tok");
    let mut s = Store::open_in_memory().unwrap();
    s.register_project("p1", "C:/r", Some("owner/repo")).unwrap();
    s.set_server_id("p1", "srv_1").unwrap();
    let local = chain_of_ruleset(3, "old-0")
        .into_iter()
        .map(|mut e| {
            e.project = Some("p1".into());
            e
        })
        .collect::<Vec<_>>();
    let mut prev = GENESIS_HASH.to_string();
    let local: Vec<_> = local
        .into_iter()
        .map(|mut e| {
            e.prev_hash = prev.clone();
            e.hash = tally_scoring::ledger::entry_hash(&e);
            prev = e.hash.clone();
            e
        })
        .collect();
    s.append_ledger(&local, 5, 0, &[]).unwrap();
    let published = tally_daemon::published::pending(&s).unwrap();
    f.set_chain("srv_1", "old-0", published[..1].iter().map(|r| r.entry.clone()).collect());
    let out = sync_all(&mut s, &f.url, &tokens());
    assert!(matches!(&out[..], [(_, Ok(c))] if c.ruleset == "old-0" && c.uploaded == 2), "{out:?}");
    assert_eq!(f.chain("srv_1", "old-0").len(), 3);
}

#[test]
fn a_malformed_ruleset_is_an_error_not_a_panic() {
    let f = FakeServer::start("tok");
    assert!(matches!(Api::new(&f.url, "tok").head("srv_1", "a&b"), Err(ApiError::Rejected { .. })));
}

#[test]
fn two_server_ids_for_one_public_name_is_refused() {
    let f = FakeServer::start("tok");
    let mut s = Store::open_in_memory().unwrap();
    s.register_project("id1", "C:/a", Some("owner/repo")).unwrap();
    s.register_project("id2", "C:/b", Some("owner/repo")).unwrap();
    s.set_server_id("id1", "srv_1").unwrap();
    s.set_server_id("id2", "srv_2").unwrap();
    s.append_events(&[prompt("a", 0, "id1")]).unwrap();
    s.set_watermark("src", 1_000_000).unwrap();
    finalize(&mut s).unwrap();
    let out = sync_all(&mut s, &f.url, &tokens());
    assert!(matches!(&out[..], [(_, Err(SyncError::ConflictingLink { project }))] if project == "owner/repo"), "{out:?}");
    assert!(f.requests().is_empty());
}

#[test]
fn sync_errors_never_contain_the_token() {
    let f = FakeServer::start("tok");
    f.revoke("srv_1");
    let mut s = two_project_store(None);
    let t = BTreeMap::from([("srv_1".into(), "tok_SECRET".into()), ("srv_2".into(), "tok2".into())]);
    let out = sync_all(&mut s, &f.url, &t);
    assert!(out.iter().all(|r| !format!("{r:?}").contains("tok_SECRET")));
}

// ---- ticker (Task 7) ----
use tally_daemon::sync::{commit_heads, SyncTicker, BACKOFF_BASE_MS, BACKOFF_MAX_MS};
use tally_scoring::ruleset::COMMIT_INTERVAL_MS;

/// Adds `n` finalized, published prompts to owner/repo (p1, linked to srv_1), ids from `from`.
fn add_published(s: &mut Store, from: usize, n: usize) {
    // later batches start well past the previous watermark, so they are not late
    let ts = |i: usize| (from * 10_000_000 + i * 100_000) as i64;
    let evs: Vec<Event> = (from..from + n).map(|i| prompt(&format!("e{i}"), ts(i), "p1")).collect();
    s.append_events(&evs).unwrap();
    s.set_watermark("src", ts(from + n) + 1_000_000).unwrap();
    finalize(s).unwrap();
    tally_daemon::published::sync(s, true).unwrap();
}

fn linked_store(n: usize) -> Store {
    let mut s = Store::open_in_memory().unwrap();
    s.register_project("p1", "C:/r", Some("owner/repo")).unwrap();
    s.set_server_id("p1", "srv_1").unwrap();
    add_published(&mut s, 0, n);
    s
}

fn local(s: &Store) -> Vec<tally_scoring::LedgerEntry> {
    s.published_rows("owner/repo").unwrap().into_iter().map(|r| r.entry).collect()
}

fn logged_in_home() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    tally_daemon::credentials::save_token(d.path(), "srv_1", "tok").unwrap();
    d
}

#[test]
fn offline_for_days_then_back_uploads_everything_in_one_tick() {
    let f = FakeServer::start("tok");
    let home = logged_in_home();
    let mut s = linked_store(1200);
    assert_eq!(local(&s).len(), 1200);
    f.down(true);
    let mut t = SyncTicker::new();
    let mut attempts = vec![];
    let mut now = 0;
    while now <= 3 * 86_400_000 {
        if t.tick(&mut s, &f.url, home.path(), now) {
            attempts.push(now);
        }
        now += COMMIT_INTERVAL_MS;
    }
    assert!(f.requests().is_empty());
    let gaps: Vec<i64> = attempts.windows(2).map(|w| w[1] - w[0]).collect();
    assert!(gaps.iter().all(|g| *g <= BACKOFF_MAX_MS), "{gaps:?}");
    assert_eq!(*gaps.last().unwrap(), BACKOFF_MAX_MS); // it did back off
    assert!(attempts.len() < 100, "{}", attempts.len());
    f.down(false);
    while !t.tick(&mut s, &f.url, home.path(), now) {
        now += COMMIT_INTERVAL_MS;
    }
    assert_eq!(f.chain("srv_1", R), local(&s));
    assert_eq!(upload_sizes(&f), [500, 500, 200]);
    assert_eq!(f.commits("srv_1").len(), 1);
}

#[test]
fn the_commit_carries_the_local_clock_unchanged_and_only_new_heads() {
    let f = FakeServer::start("tok");
    let home = logged_in_home();
    let mut s = linked_store(3);
    let mut t = SyncTicker::new();
    assert!(t.tick(&mut s, &f.url, home.path(), 42)); // a clock far behind the server's
    let l = local(&s);
    assert_eq!(f.commits("srv_1"), [(l[2].hash.clone(), 42)]);
    assert!(t.tick(&mut s, &f.url, home.path(), 42 + COMMIT_INTERVAL_MS));
    assert_eq!(f.commits("srv_1").len(), 1);
    add_published(&mut s, 3, 2);
    assert!(t.tick(&mut s, &f.url, home.path(), 42 + 2 * COMMIT_INTERVAL_MS));
    let l = local(&s);
    assert_eq!(l.len(), 5);
    let c = f.commits("srv_1");
    assert_eq!(c.len(), 2);
    assert_eq!(c[1], (l[4].hash.clone(), 42 + 2 * COMMIT_INTERVAL_MS));
    assert_eq!(f.chain("srv_1", R), l);
}

#[test]
fn a_tick_before_the_interval_does_no_io() {
    let f = FakeServer::start("tok");
    let home = logged_in_home();
    let mut s = linked_store(1);
    let mut t = SyncTicker::new();
    assert!(t.tick(&mut s, &f.url, home.path(), 0));
    let n = f.requests().len();
    assert!(n > 0);
    assert!(!t.tick(&mut s, &f.url, home.path(), COMMIT_INTERVAL_MS - 1));
    assert_eq!(f.requests().len(), n);
}

#[test]
fn a_rejected_token_backs_off_for_an_hour() {
    let f = FakeServer::start("tok");
    let home = logged_in_home();
    let mut s = linked_store(1);
    f.revoke("srv_1");
    let mut t = SyncTicker::new();
    assert!(t.tick(&mut s, &f.url, home.path(), 0));
    assert!(!t.tick(&mut s, &f.url, home.path(), BACKOFF_MAX_MS - 1));
    assert!(t.tick(&mut s, &f.url, home.path(), BACKOFF_MAX_MS));
}

#[test]
fn the_run_loop_syncs_only_when_enabled() {
    use tally_daemon::run::{hooks, run_once, sync_hook};
    let f = FakeServer::start("tok");
    let home = logged_in_home();
    let mut cfg = tally_daemon::config::Config::default();
    cfg.publish.enabled = false;
    cfg.sync.server = Some(f.url.clone());
    assert!(sync_hook(&cfg, home.path()).is_none()); // server set, not enabled
    assert_eq!(hooks(&cfg, home.path()).len(), 1);
    cfg.sync.enabled = true;
    cfg.sync.server = None;
    assert!(sync_hook(&cfg, home.path()).is_none());
    cfg.sync.server = Some(f.url.clone());
    let mut hks = hooks(&cfg, home.path());
    assert_eq!(hks.len(), 2);
    let mut s = linked_store(3);
    let r = run_once(&mut s, &mut [], &mut hks, 1_000);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(f.chain("srv_1", R), local(&s));
    assert_eq!(f.commits("srv_1"), [(local(&s)[2].hash.clone(), 1_000)]);
}

#[test]
fn a_tick_with_only_permanent_errors_resets_the_backoff() {
    let f = FakeServer::start("tok");
    let home = logged_in_home();
    let mut s = linked_store(1);
    // owner/other is linked and published but has no token: NotLoggedIn on every tick
    s.register_project("p2", "C:/o", Some("owner/other")).unwrap();
    s.set_server_id("p2", "srv_2").unwrap();
    s.append_events(&[prompt("o1", 5_000_000, "p2")]).unwrap();
    s.set_watermark("src", 7_000_000).unwrap();
    finalize(&mut s).unwrap();
    tally_daemon::published::sync(&mut s, true).unwrap();
    let mut t = SyncTicker::new();
    f.down(true);
    let mut now = 0;
    for _ in 0..2 {
        assert!(t.tick(&mut s, &f.url, home.path(), now));
        now += COMMIT_INTERVAL_MS;
    }
    f.down(false);
    assert!(t.tick(&mut s, &f.url, home.path(), now)); // owner/repo syncs, owner/other is NotLoggedIn
    assert_eq!(f.chain("srv_1", R).len(), 1);
    f.down(true);
    now += COMMIT_INTERVAL_MS;
    assert!(t.tick(&mut s, &f.url, home.path(), now));
    assert_eq!(t.backoff().next_ms() - now, BACKOFF_BASE_MS); // a fresh outage starts at 30 s again
}

#[test]
fn commit_heads_skips_projects_without_a_token() {
    let f = FakeServer::start("tok");
    let s = linked_store(1);
    let mut last = BTreeMap::new();
    assert!(commit_heads(&s, &f.url, &BTreeMap::new(), &mut last, 1).is_empty());
    assert!(f.requests().is_empty());
    let t = BTreeMap::from([("srv_1".to_string(), "tok".to_string())]);
    let out = commit_heads(&s, &f.url, &t, &mut last, 1);
    assert!(matches!(&out[..], [(p, Ok(()))] if p == "owner/repo"), "{out:?}");
    assert_eq!(last["srv_1"], local(&s)[0].hash);
}

// ---- privacy guard (Task 8) ----
// Enforces the first Global Constraint: only ledger entries and head hashes leave the machine.
// When a later plan adds a field to a request, change this test on purpose.
#[test]
fn nothing_but_ledger_entries_and_head_hashes_leaves_the_machine() {
    use std::collections::BTreeSet;
    use tally_scoring::LedgerEntry;
    const TOKEN: &str = "tok_SECRET_beyond_header";
    const ID1: &str = "LOCALID_one_0123456789abcdef";
    const ID2: &str = "LOCALID_two_0123456789abcdef";

    let f = FakeServer::start(TOKEN);
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("config.toml"), format!("[sync]\nenabled = true\nserver = '{}'\n", f.url)).unwrap();
    let bin = |args: &[&str], stdin: Option<&str>| {
        use std::io::Write;
        use std::process::{Command, Stdio};
        let mut c = Command::new(env!("CARGO_BIN_EXE_tally"))
            .arg("--home").arg(&home).args(args)
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
        let mut si = c.stdin.take().unwrap();
        if let Some(s) = stdin {
            si.write_all(s.as_bytes()).unwrap();
        }
        drop(si);
        let o = c.wait_with_output().unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    };
    bin(&["login", "--server-id", "srv_1"], Some(&format!("{TOKEN}\n")));

    let repo = t.path().join("PRIVATE_PATH_MARKER").join("repo");
    let other = t.path().join("OTHER_PROJECT_MARKER");
    for d in [&repo, &other] {
        std::fs::create_dir_all(d.join(".git")).unwrap();
    }
    let mut s = Store::open(&home.join("tally.db")).unwrap();
    s.register_project(ID1, repo.to_str().unwrap(), Some("owner/repo")).unwrap();
    s.set_server_id(ID1, "srv_1").unwrap();
    s.register_project(ID2, other.to_str().unwrap(), None).unwrap();

    // prompt, edit and idle for both projects; batch `b` starts well past the previous watermark
    let batch = |s: &mut Store, b: i64| {
        let base = b * 10_000_000;
        let mut evs = vec![];
        for (n, id) in [(0, ID1), (1, ID2)] {
            let ts = base + n * 1_000;
            let mut edit = prompt(&format!("u{b}{n}"), ts + 10_000, id);
            edit.kind = Kind::UserEdit { file_hash: "ab".repeat(32), lines_changed: 3 };
            let mut idle = prompt(&format!("i{b}{n}"), ts + 20_000, id);
            idle.kind = Kind::Idle { end_ts: ts + 1_000_000 };
            evs.extend([prompt(&format!("p{b}{n}"), ts, id), edit, idle]);
        }
        s.append_events(&evs).unwrap();
        s.set_watermark("src", base + 5_000_000).unwrap();
        finalize(s).unwrap();
        tally_daemon::published::sync(s, true).unwrap();
    };
    batch(&mut s, 0);
    let mut tk = SyncTicker::new();
    assert!(tk.tick(&mut s, &f.url, &home, 0));
    batch(&mut s, 1);
    assert!(tk.tick(&mut s, &f.url, &home, COMMIT_INTERVAL_MS));
    batch(&mut s, 2);
    drop(s);
    let before = upload_sizes(&f).len();
    bin(&["sync"], None);
    assert!(upload_sizes(&f).len() > before, "the binary should have uploaded the last batch");

    let reqs = f.requests();
    let entry_keys: BTreeSet<&str> = ["v", "ruleset", "action_id", "ts", "project", "kind", "verified_s",
        "unverified_s", "reasons", "features", "prev_hash", "hash"].into();
    let allowed: BTreeSet<&str> = ["host", "user-agent", "authorization", "content-type", "content-length",
        "accept", "accept-encoding", "connection"].into();
    let keys = |v: &serde_json::Value| -> BTreeSet<String> { v.as_object().unwrap().keys().cloned().collect() };
    let set = |a: &[&str]| -> BTreeSet<String> { a.iter().map(|s| s.to_string()).collect() };
    for r in &reqs {
        let everything = format!("{} {} {:?} {}", r.method, r.path, r.headers, r.body);
        // the token is only ever the authorization header value
        let without_auth = format!("{} {} {:?} {}", r.method, r.path,
            r.headers.iter().filter(|(k, _)| k != "authorization").collect::<Vec<_>>(), r.body);
        assert!(!without_auth.contains("tok_SECRET"), "token leaked outside authorization: {without_auth}");
        assert!(r.headers.iter().any(|(k, v)| k == "authorization" && v == &format!("Bearer {TOKEN}")));
        for secret in ["PRIVATE_PATH_MARKER", "OTHER_PROJECT_MARKER", "LOCALID_"] {
            assert!(!everything.contains(secret), "{secret} leaked in {everything}");
        }
        let names: BTreeSet<&str> = r.headers.iter().map(|(k, _)| k.as_str()).collect();
        assert!(names.is_subset(&allowed), "{names:?}");
        assert!(r.headers.iter().any(|(k, v)| k == "user-agent" && v.starts_with("tally/") && !v.contains(' ')));
        match (r.method.as_str(), r.path.rsplit('/').next().unwrap().split('?').next().unwrap()) {
            ("GET", "head") => assert!(r.body.is_empty()),
            ("POST", "entries") => {
                let v: serde_json::Value = serde_json::from_str(&r.body).unwrap();
                assert_eq!(keys(&v), set(&["entries", "prev_hash"]));
                for e in v["entries"].as_array().unwrap() {
                    assert_eq!(keys(e), entry_keys.iter().map(|s| s.to_string()).collect());
                    assert_eq!(e["project"], "owner/repo"); // public name, never the server id
                    let _: LedgerEntry = serde_json::from_value(e.clone()).unwrap();
                }
            }
            ("POST", "commit") => {
                let v: serde_json::Value = serde_json::from_str(&r.body).unwrap();
                assert_eq!(keys(&v), set(&["hash", "ts"]));
                assert_eq!(v["hash"].as_str().unwrap().len(), 64);
            }
            other => panic!("unexpected request {other:?}"),
        }
    }
    assert!(reqs.iter().any(|r| r.path.contains("/entries")) && reqs.iter().any(|r| r.path.contains("/commit")));
}

/// SYNC-2: an unreachable server must stop the pass after the first failure, not cost every project a timeout.
#[test]
fn sync_stops_after_the_first_network_failure() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    let hits = Arc::new(AtomicUsize::new(0));
    let h = hits.clone();
    std::thread::spawn(move || {
        for c in l.incoming() {
            h.fetch_add(1, Ordering::SeqCst);
            drop(c); // accept and hang up: a network error
        }
    });
    let mut s = tally_daemon::store::Store::open_in_memory().unwrap();
    let mut evs = vec![];
    for i in 0..3 {
        let p = format!("p{i}");
        s.register_project(&p, &format!("C:/r{i}"), Some(&format!("qa/proj{i}"))).unwrap();
        s.set_server_id(&p, &format!("srv_{i}")).unwrap();
        evs.push(tally_scoring::Event {
            v: 1, id: format!("a{i}"), ts: (i as i64) * 100_000, source: "test".into(), project: Some(p),
            session: None, kind: tally_scoring::Kind::Prompt { chars: 200, relevance_pm: 600, novelty_bytes: 100 },
        });
    }
    s.append_events(&evs).unwrap();
    s.set_watermark("src", 100_000_000).unwrap();
    tally_daemon::finalize::finalize(&mut s).unwrap();
    let tokens = (0..3).map(|i| (format!("srv_{i}"), "tok".to_string())).collect();
    let (commits, uploads) =
        tally_daemon::sync::sync_once(&mut s, &url, &tokens, &mut Default::default(), Some(1_000_000));
    std::thread::sleep(std::time::Duration::from_millis(200));
    assert_eq!(commits.len() + uploads.len(), 1, "{commits:?} {uploads:?}");
    assert!(hits.load(Ordering::SeqCst) <= 2, "hits {}", hits.load(Ordering::SeqCst));
}

#[test]
fn a_429_between_batches_is_waited_out_not_fatal() {
    let f = FakeServer::start("tok");
    f.max_batch(1);
    // The second batch (after the first succeeded) gets a 429; the sync must retry it.
    f.on_next_upload(|s| s.fail = (1, 429));
    let r = sync_chain(&api(&f), "srv_1", "p", R, &chain_of(2)).unwrap();
    assert_eq!(r.uploaded, 2);
}
