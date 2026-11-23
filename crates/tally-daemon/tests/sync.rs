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
