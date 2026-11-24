//! Drives the real binary: `tally dashboard` prints its URL and answers on loopback.
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use tally_scoring::{Event, Kind};

struct Guard(Child);
impl Drop for Guard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn tally(home: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_tally")).arg("--home").arg(home).args(args).output().unwrap()
}

fn start(home: &Path) -> (Guard, u16) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_tally"))
        .arg("--home")
        .arg(home)
        .args(["dashboard", "--port", "0", "--no-open"])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
    let guard = Guard(child);
    let port = line.trim().rsplit(':').next().unwrap().trim_end_matches('/').parse().unwrap_or_else(|_| panic!("no url in {line:?}"));
    (guard, port)
}

fn get(port: u16, path: &str) -> String {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(s, "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n").unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    out
}

fn ev(id: &str, secs: i64, kind: Kind) -> Event {
    Event { v: 1, id: id.into(), ts: secs * 1000, source: "test".into(), project: Some("p1".into()), session: None, kind }
}

#[test]
fn tally_dashboard_prints_its_url_and_serves_status() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    let file = t.path().join("events.jsonl");
    let evs = [
        ev("1", 0, Kind::Prompt { chars: 200, relevance_pm: 600, novelty_bytes: 100 }),
        ev("2", 100, Kind::UserEdit { file_hash: "f".into(), lines_changed: 2 }),
        ev("3", 200, Kind::Prompt { chars: 200, relevance_pm: 600, novelty_bytes: 100 }),
    ];
    let text: Vec<String> = evs.iter().map(|e| serde_json::to_string(e).unwrap()).collect();
    std::fs::write(&file, text.join("\n") + "\n").unwrap();
    assert!(tally(&home, &["ingest", file.to_str().unwrap(), "--watermark", "3000000"]).status.success());
    assert!(tally(&home, &["finalize"]).status.success());
    let (_g, port) = start(&home);
    let r = get(port, "/api/status");
    assert!(r.starts_with("HTTP/1.1 200"), "{r}");
    let body: serde_json::Value = serde_json::from_str(r.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(body["data"]["final_claims"], 3);
    assert!(get(port, "/").contains(r#"id="app""#));
}

#[test]
fn dashboard_without_a_database_still_serves_the_page_and_503s_the_api() {
    let t = tempfile::tempdir().unwrap();
    let (_g, port) = start(&t.path().join("empty"));
    assert!(get(port, "/").starts_with("HTTP/1.1 200"));
    assert!(get(port, "/api/status").starts_with("HTTP/1.1 503"));
}
