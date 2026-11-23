use super::*;
use crate::finalize::finalize;
use crate::test_support::{ev, prompt};
use tally_scoring::{Actor, Kind};

const T: i64 = 1_700_000_000_000;
const CMD: &str = "SECRETCMDHASH0123456789abcdef";
const FILE: &str = "SECRETFILEHASH0123456789abcdef";
const HOST: &str = "127.0.0.1:7878";

fn run(id: &str, ms: i64, exit: i32) -> tally_scoring::Event {
    ev(id, T + ms, Kind::TestRun { cmd_hash: CMD.into(), exit_code: exit, initiator: Actor::User })
}
fn fixture() -> (tempfile::TempDir, DashApi) {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("tally.db");
    let mut s = Store::open(&path).unwrap();
    let mut p2 = prompt("c", T + 200_000, 500);
    p2.project = Some("p2".into());
    s.append_events(&[
        prompt("a", T, 600),
        run("f", 10_000, 1),
        ev("b", T + 60_000, Kind::UserEdit { file_hash: FILE.into(), lines_changed: 3 }),
        run("ok", 120_000, 0),
        p2,
    ])
    .unwrap();
    s.set_watermark("src", T + 10_000_000).unwrap();
    finalize(&mut s).unwrap();
    s.conn.execute("INSERT INTO skipped_claims VALUES ('x', ?1, 'no_presence_coverage')", [T + 300_000]).unwrap();
    (d, DashApi::new(path, 7878))
}
fn get(api: &DashApi, method: &str, path: &str, query: &str, host: Option<&str>) -> Response {
    api.handle(&Request { method, path, query, host })
}
fn ok(api: &DashApi, path: &str, query: &str) -> serde_json::Value {
    let r = get(api, "GET", path, query, Some(HOST));
    assert_eq!(r.status, 200, "{path}: {}", String::from_utf8_lossy(&r.body));
    serde_json::from_slice(&r.body).unwrap()
}
const Q: &str = "from=1699999999000&to=1700100000000";

#[test]
fn foreign_host_is_403_and_never_opens_the_database() {
    let api = DashApi::new("/nonexistent/tally.db".into(), 7878);
    for h in [Some("evil.example:7878"), Some("127.0.0.1:7879"), None] {
        assert_eq!(get(&api, "GET", "/api/status", "", h).status, 403, "{h:?}");
    }
    assert_eq!(get(&api, "GET", "/api/status", "", Some("LOCALHOST:7878")).status, 503);
}
#[test]
fn only_get_and_head_are_routed_and_no_cors_headers_ever() {
    let (_d, api) = fixture();
    for m in ["POST", "PUT", "DELETE", "OPTIONS"] {
        let r = get(&api, m, "/api/status", "", Some(HOST));
        assert_eq!((r.status, r.allow), (405, Some("GET, HEAD")));
        assert!(r.headers().iter().all(|(k, _)| !k.to_lowercase().starts_with("access-control")));
    }
    assert_eq!(get(&api, "HEAD", "/api/status", "", Some(HOST)).status, 200);
    let r = get(&api, "GET", "/api/status", "", Some(HOST));
    let h = r.headers();
    assert!(h.iter().any(|(k, v)| *k == "Content-Security-Policy" && v == CSP));
    assert!(h.iter().any(|(k, v)| *k == "Cache-Control" && v == "no-store"));
    assert_eq!(get(&api, "GET", "/api/nope", "", Some(HOST)).status, 404);
}
#[test]
fn status_reports_counts_sources_and_projects_with_as_of() {
    let (_d, api) = fixture();
    let v = ok(&api, "/api/status", "");
    assert_eq!(v["as_of"]["horizon"], T + 10_000_000);
    assert_eq!(v["as_of"]["finalized_through"], T + 10_000_000);
    let d = &v["data"];
    assert_eq!((d["events"].as_u64(), d["final_claims"].as_u64(), d["skipped_claims"].as_u64()), (Some(5), Some(5), Some(1)));
    assert_eq!(d["sources"][0]["source"], "src");
}
#[test]
fn analytics_routes_return_the_analytics_types() {
    let (_d, api) = fixture();
    assert_eq!(ok(&api, "/api/bugs", Q)["data"]["fixed"], 1);
    assert_eq!(ok(&api, "/api/prompts", Q)["data"]["informed_pm_threshold"], 300);
    assert_eq!(ok(&api, "/api/sessions", Q)["data"].as_array().unwrap().len(), 2);
    let edges = format!("edges={},{}", T - 1000, T + 100_000_000);
    let h = ok(&api, "/api/hours", &edges);
    assert_eq!(h["data"]["totals"]["claims"], 5);
    assert_eq!(h["data"]["no_presence"]["claims"], 1);
    assert_eq!(h["data"]["edges"].as_array().unwrap().len(), 2);
    let r = ok(&api, "/api/replay", Q);
    let kinds: Vec<&str> = r["data"]["items"].as_array().unwrap().iter().map(|i| i["kind"].as_str().unwrap()).collect();
    assert!(kinds.contains(&"fix") && kinds.contains(&"skipped"), "{kinds:?}");
    let only = ok(&api, "/api/replay", &format!("{Q}&project=p2"));
    assert_eq!(only["data"]["items"].as_array().unwrap().len(), 1);
}
#[test]
fn bad_parameters_are_400_with_a_reason() {
    let (_d, api) = fixture();
    let many = (0..368).map(|i| i.to_string()).collect::<Vec<_>>().join(",");
    let cases = [
        ("/api/bugs", "from=abc&to=5".to_string(), "from"),
        ("/api/bugs", "from=5&to=5".to_string(), "from"),
        ("/api/bugs", format!("from=0&to={}", 400 * DAY_MS), "366"),
        ("/api/hours", "edges=5,3".to_string(), "edges"),
        ("/api/hours", format!("edges={many}"), "edges"),
        ("/api/hours", "edges=1, 2".to_string(), "edges"),
        ("/api/hours", "edges=1%2C2".to_string(), "edges"),
        ("/api/replay", format!("{Q}&after=nocolon"), "after"),
    ];
    for (path, q, word) in cases {
        let r = get(&api, "GET", path, &q, Some(HOST));
        let body = String::from_utf8_lossy(&r.body).into_owned();
        assert!(r.status == 400 && body.contains(word), "{path}?{q}: {} {body}", r.status);
    }
    assert_eq!(get(&api, "GET", "/api/sessions", &format!("{Q}&limit=99999&bogus=1"), Some(HOST)).status, 200);
}
#[test]
fn missing_database_is_503_not_a_panic_or_a_500() {
    let api = DashApi::new("/nonexistent/tally.db".into(), 7878);
    assert_eq!(get(&api, "GET", "/api/status", "", Some(HOST)).status, 503);
}
#[test]
fn no_response_contains_hashes_or_text_from_the_fixture() {
    let (_d, api) = fixture();
    let edges = format!("edges={},{}", T - 1000, T + 100_000_000);
    for (path, q) in [("/api/status", ""), ("/api/bugs", Q), ("/api/prompts", Q), ("/api/sessions", Q), ("/api/replay", Q), ("/api/hours", edges.as_str())] {
        let body = String::from_utf8(get(&api, "GET", path, q, Some(HOST)).body).unwrap();
        assert!(!body.contains("SECRETCMDHASH0123456789") && !body.contains("SECRETFILEHASH"), "{path}");
    }
}
#[test]
fn a_100k_event_store_answers_every_route_under_one_second() {
    if std::env::var_os("TALLY_SLOW_TESTS").is_none() {
        return;
    }
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("tally.db");
    let s = Store::open(&path).unwrap();
    s.conn.execute_batch("BEGIN").unwrap();
    for i in 0..100_000i64 {
        let e = if i % 100 == 0 { run(&format!("r{i}"), i * 24, (i % 3 == 0) as i32) } else { prompt(&format!("p{i}"), T + i * 24, 400) };
        s.conn.execute("INSERT INTO events(id, ts, json) VALUES (?1, ?2, ?3)", rusqlite::params![e.id, e.ts, serde_json::to_string(&e).unwrap()]).unwrap();
    }
    s.conn.execute_batch("COMMIT").unwrap();
    drop(s);
    let api = DashApi::new(path, 7878);
    let edges = format!("edges={},{}", T, T + 28 * DAY_MS);
    let q = format!("from={T}&to={}", T + 28 * DAY_MS);
    for (p, q) in [("/api/status", String::new()), ("/api/bugs", q.clone()), ("/api/prompts", q.clone()), ("/api/hours", edges), ("/api/replay", q)] {
        let t = std::time::Instant::now();
        assert_eq!(get(&api, "GET", p, &q, Some(HOST)).status, 200);
        // plan says 1 s; an unoptimized build measured 1.4 s for /api/bugs (every event in the window), so debug gets 5 s
        assert!(t.elapsed().as_secs_f64() < if cfg!(debug_assertions) { 5.0 } else { 1.0 },"{p} took {:?}", t.elapsed());
    }
}
