use base64::Engine;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use tally_daemon::heartbeat::server::{host_ok, start, Received, Running, ServerConfig};

const KEY: &str = "test-key";
const SINGLE: &str = r#"{"entity":"C:/p/a.rs","type":"file","time":1763800000.5,"is_write":true}"#;

fn up() -> (Running, Arc<Mutex<Vec<Received>>>) {
    let sink = Arc::new(Mutex::new(Vec::new()));
    (start(ServerConfig::new(0, KEY.into()), sink.clone()).unwrap(), sink)
}

fn auth() -> String {
    format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(KEY))
}

/// Returns (status, body).
fn req(r: &Running, method: &str, path: &str, host: &str, auth: Option<&str>, body: &str) -> (u16, String) {
    let mut s = TcpStream::connect(r.addr).unwrap();
    let mut m = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nContent-Length: {}\r\n", body.len());
    if let Some(a) = auth {
        m += &format!("Authorization: {a}\r\n");
    }
    s.write_all(format!("{m}\r\n{body}").as_bytes()).unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    let code = out.split(' ').nth(1).unwrap().parse().unwrap();
    (code, out.split_once("\r\n\r\n").map(|x| x.1).unwrap_or("").to_string())
}

fn post(r: &Running, path: &str, body: &str) -> (u16, String) {
    req(r, "POST", path, "localhost", Some(&auth()), body)
}

const ONE: &str = "/api/v1/users/current/heartbeats";
const BULK: &str = "/api/v1/users/current/heartbeats.bulk";

#[test]
fn binds_loopback_only() {
    let (r, _s) = up();
    assert!(r.addr.ip().is_loopback());
    assert_eq!(r.addr.ip().to_string(), "127.0.0.1");
}

#[test]
fn foreign_host_header_is_403() {
    let (r, s) = up();
    assert_eq!(req(&r, "POST", ONE, "evil.example", Some(&auth()), SINGLE).0, 403);
    assert!(s.lock().unwrap().is_empty());
    assert!(!host_ok(None) && !host_ok(Some("127.0.0.1.evil.example")) && !host_ok(Some("[::1]x")));
}

#[test]
fn localhost_host_with_port_is_accepted() {
    let (r, _s) = up();
    let h = format!("localhost:{}", r.addr.port());
    assert_eq!(req(&r, "POST", ONE, &h, Some(&auth()), SINGLE).0, 201);
    assert!(host_ok(Some("[::1]:25930")) && host_ok(Some("127.0.0.1:1")));
}

#[test]
fn missing_or_wrong_key_is_401() {
    let (r, s) = up();
    assert_eq!(req(&r, "POST", ONE, "localhost", None, SINGLE), (401, String::new()));
    assert_eq!(req(&r, "POST", ONE, "localhost", Some("Bearer nope"), SINGLE).0, 401);
    assert_eq!(req(&r, "POST", &format!("{ONE}?api_key={KEY}"), "localhost", None, SINGLE).0, 201);
    assert_eq!(s.lock().unwrap().len(), 1);
}

#[test]
fn single_heartbeat_gets_201_data() {
    let (r, _s) = up();
    let (c, b) = post(&r, ONE, SINGLE);
    let v: serde_json::Value = serde_json::from_str(&b).unwrap();
    assert_eq!((c, v["data"]["entity"].as_str()), (201, Some("C:/p/a.rs")));
    assert_eq!(post(&r, "/api/hackatime/v1/users/current/heartbeats", SINGLE).0, 201);
}

#[test]
fn bulk_gets_201_responses_in_order_with_per_item_status() {
    let (r, _s) = up();
    let body = format!(r#"[{SINGLE},{{"entity":"x"}},{SINGLE}]"#);
    let (c, b) = post(&r, BULK, &body);
    let v: serde_json::Value = serde_json::from_str(&b).unwrap();
    let codes: Vec<_> = v["responses"].as_array().unwrap().iter().map(|p| p[1].as_u64().unwrap()).collect();
    assert_eq!((c, codes), (201, vec![201, 400, 201]));
}

#[test]
fn oversized_content_length_is_413() {
    let (r, _s) = up();
    let mut s = TcpStream::connect(r.addr).unwrap();
    let m = format!("POST {ONE} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nAuthorization: {}\r\nContent-Length: 999999\r\n\r\n", auth());
    s.write_all(m.as_bytes()).unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    assert!(out.starts_with("HTTP/1.1 413"), "{out}");
}

#[test]
fn bulk_over_25_is_400() {
    let (r, s) = up();
    let body = format!("[{}]", vec![SINGLE; 26].join(","));
    assert_eq!(post(&r, BULK, &body).0, 400);
    assert!(s.lock().unwrap().is_empty());
    assert_eq!(post(&r, BULK, &format!("[{}]", vec![SINGLE; 25].join(","))).0, 201);
}

#[test]
fn get_is_405() {
    let (r, _s) = up();
    assert_eq!(req(&r, "GET", ONE, "localhost", Some(&auth()), "").0, 405);
}

#[test]
fn unknown_path_is_404() {
    let (r, _s) = up();
    assert_eq!(post(&r, "/api/v1/users/current/other", SINGLE).0, 404);
}

#[test]
fn sink_receives_only_valid_heartbeats() {
    let (r, s) = up();
    post(&r, BULK, &format!(r#"[{SINGLE},{{"entity":"x"}},{SINGLE}]"#));
    post(&r, ONE, "{nope");
    assert_eq!(s.lock().unwrap().len(), 2);
}

#[test]
fn end_to_end_post_then_poll_emits_one_user_edit() {
    use std::collections::BTreeMap;
    use tally_daemon::heartbeat::server::ServerConfig;
    use tally_daemon::heartbeat::HeartbeatSource;
    use tally_daemon::source::Source;
    let d = tempfile::tempdir().unwrap();
    let repo = d.path().join("repo");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    std::fs::create_dir_all(repo.join("src")).unwrap();
    let file = repo.join("src").join("uniquepathname.rs");
    let esc = file.to_string_lossy().replace('\\', "\\\\");
    let t = 1_763_800_000.0;
    let body = format!(
        r#"[{{"entity":"{esc}","type":"file","time":{t},"is_write":true}},{{"entity":"{esc}","type":"file","time":{},"is_write":true}}]"#,
        t + 5.0
    );
    let mut src = HeartbeatSource::start_with(ServerConfig::new(0, KEY.into()), vec![]).unwrap();
    let addr = src.addr().unwrap();
    let mut s = TcpStream::connect(addr).unwrap();
    let m = format!(
        "POST {BULK} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nAuthorization: {}\r\nContent-Length: {}\r\n\r\n{body}",
        auth(),
        body.len()
    );
    s.write_all(m.as_bytes()).unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    assert!(out.starts_with("HTTP/1.1 201"), "{out}");
    let p = src.poll(1_763_800_000_000 + 60_000, &mut BTreeMap::<String, String>::new()).unwrap();
    assert_eq!(p.events.len(), 1);
    // Persist and make sure the path text never reaches the db file.
    let db = d.path().join("tally.db");
    let mut store = tally_daemon::store::Store::open(&db).unwrap();
    store.append_events(&p.events).unwrap();
    drop(store);
    let bytes: Vec<u8> = ["", "-wal"].iter().flat_map(|x| std::fs::read(format!("{}{x}", db.display())).unwrap_or_default()).collect();
    assert!(!bytes.windows(14).any(|w| w == b"uniquepathname"));
}
