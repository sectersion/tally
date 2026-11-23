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
