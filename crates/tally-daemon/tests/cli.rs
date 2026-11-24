use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tally_scoring::{Event, Kind};

fn tally(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tally")).arg("--home").arg(home).args(args).env_remove("CLAUDECODE").output().unwrap()
}
fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).replace("\r\n", "\n").trim().to_string()
}
fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).to_string()
}
fn ev(id: &str, secs: i64, kind: Kind) -> Event {
    Event { v: 1, id: id.into(), ts: secs * 1000, source: "test".into(), project: None, session: None, kind }
}
fn prompt() -> Kind {
    Kind::Prompt { chars: 200, relevance_pm: 600, novelty_bytes: 100 }
}
fn write_events(path: &Path, events: &[Event]) {
    let text: Vec<String> = events.iter().map(|e| serde_json::to_string(e).unwrap()).collect();
    std::fs::write(path, text.join("\n") + "\n").unwrap();
}

#[test]
fn lunch_scenario_end_to_end() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    let file = t.path().join("events.jsonl");
    write_events(&file, &[ev("1", 0, prompt()), ev("2", 60, Kind::Idle { end_ts: 1_860_000 }), ev("3", 1900, prompt())]);
    let f = file.to_str().unwrap();

    let o = tally(&home, &["ingest", f, "--watermark", "3000000"]);
    assert!(o.status.success(), "{}", err(&o));
    assert_eq!(out(&o), "added 3, duplicates 0");

    let o = tally(&home, &["finalize"]);
    assert!(o.status.success(), "{}", err(&o));
    assert_eq!(out(&o), "appended 2, skipped 0, final 2, provisional 0");

    let o = tally(&home, &["status"]);
    let s = out(&o);
    assert!(s.lines().any(|l| l == "final_claims: 2"), "{s}");
    assert!(s.lines().any(|l| l == "verified_s: 660"), "{s}");
    assert!(s.lines().any(|l| l == "skipped_claims: 0"), "{s}");

    let o = tally(&home, &["verify"]);
    assert!(o.status.success());
    assert_eq!(out(&o), "ok: 2 entries");

    assert_eq!(out(&tally(&home, &["ingest", f])), "added 0, duplicates 3");

    let late = t.path().join("late.jsonl");
    write_events(&late, &[ev("4", 1000, prompt())]);
    let o = tally(&home, &["ingest", late.to_str().unwrap()]);
    assert!(!o.status.success());
    assert!(err(&o).contains("late event"), "{}", err(&o));
    let s = out(&tally(&home, &["status"]));
    assert!(s.lines().any(|l| l == "events: 3"), "{s}");

    assert_eq!(out(&tally(&home, &["finalize"])), "appended 0, skipped 0, final 2, provisional 0");
}

#[test]
fn project_add_rejects_a_directory_outside_git_and_prints_the_id_inside_one() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    let repo = t.path().join("repo");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    let o = tally(&home, &["project", "add", repo.to_str().unwrap()]);
    assert!(o.status.success(), "{}", err(&o));
    let id = out(&o);
    assert_eq!(id.len(), 64, "{id}");
    assert!(id.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)));

    let plain = tempfile::tempdir().unwrap();
    if tally_daemon::find_git_root(plain.path()).is_none() {
        let o = tally(&home, &["project", "add", plain.path().to_str().unwrap()]);
        assert!(!o.status.success());
        assert!(err(&o).contains("not inside a git repository"), "{}", err(&o));
    }
}

#[test]
fn project_link_stores_the_server_id() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    let repo = t.path().join("repo");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    let r = repo.to_str().unwrap();
    let added = tally(&home, &["project", "add", r, "--name", "owner/repo"]);
    assert!(added.status.success(), "{}", err(&added));
    let o = tally(&home, &["project", "link", r, "--server-id", "srv_1"]);
    assert!(o.status.success(), "{}", err(&o));
    assert_eq!(out(&o), out(&added));
    assert_eq!(out(&o).len(), 64);
    let c = rusqlite::Connection::open(home.join("tally.db")).unwrap();
    let (name, sid): (String, String) = c
        .query_row("SELECT public_name, server_id FROM projects WHERE local_id = ?1", [out(&o)], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap();
    assert_eq!((name.as_str(), sid.as_str()), ("owner/repo", "srv_1"));
    let bad = tally(&home, &["project", "link", r, "--server-id", "a/b"]);
    assert!(!bad.status.success());
}

#[test]
fn project_add_reports_an_adopted_name_on_stderr_only() {
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    let mk = |n: &str| {
        let d = t.path().join(n);
        std::fs::create_dir_all(&d).unwrap();
        for args in [&["init", "-q"][..], &["remote", "add", "origin", "https://github.com/o/r.git"]] {
            assert!(Command::new("git").arg("-C").arg(&d).args(args).output().unwrap().status.success());
        }
        d
    };
    let (a, b) = (mk("a"), mk("b"));
    let o = tally(&home, &["project", "add", a.to_str().unwrap(), "--name", "o/r"]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(!err(&o).contains("matched"), "{}", err(&o));
    let o = tally(&home, &["project", "add", b.to_str().unwrap()]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(err(&o).contains("matched o/r via remote"), "{}", err(&o));
    assert_eq!(out(&o).len(), 64);
}

#[test]
fn read_only_commands_need_an_existing_database_and_create_nothing() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    for cmd in ["status", "verify", "finalize"] {
        let o = tally(&home, &[cmd]);
        assert!(!o.status.success(), "{cmd}");
        assert!(err(&o).contains("no database"), "{cmd}: {}", err(&o));
        assert!(!home.join("tally.db").exists(), "{cmd}");
    }
}

#[test]
fn a_bad_event_line_names_its_line_number() {
    let t = tempfile::tempdir().unwrap();
    let file = t.path().join("bad.jsonl");
    let good = serde_json::to_string(&ev("1", 0, prompt())).unwrap();
    std::fs::write(&file, format!("{good}\nnot json\n")).unwrap();
    let o = tally(&t.path().join("home"), &["ingest", file.to_str().unwrap()]);
    assert!(!o.status.success());
    assert!(err(&o).contains("line 2"), "{}", err(&o));
}

#[test]
fn run_once_with_an_empty_projects_dir_finalizes_and_exits() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let empty = tempfile::tempdir().unwrap();
    std::fs::write(home.join("config.toml"), format!("[claude_code]\nprojects_dir = '{}'\n", empty.path().display())).unwrap();
    let o = tally(&home, &["run", "--once"]);
    assert!(o.status.success(), "{}", err(&o));
    assert_eq!(out(&o), "added 0, duplicates 0, skipped 0, final 0, provisional 0");
    assert!(home.join("tally.db").exists());
}

#[test]
fn run_refuses_a_bad_config() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("config.toml"), "[run]\nbogus = 1\n").unwrap();
    let o = tally(&home, &["run", "--once"]);
    assert!(!o.status.success());
    assert!(err(&o).contains("unknown field"), "{}", err(&o));
}

#[test]
fn verify_reports_a_tampered_ledger_entry_by_index() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    let file = t.path().join("events.jsonl");
    write_events(&file, &[ev("1", 0, prompt()), ev("2", 60, Kind::Idle { end_ts: 1_860_000 }), ev("3", 1900, prompt())]);
    assert!(tally(&home, &["ingest", file.to_str().unwrap(), "--watermark", "3000000"]).status.success());
    assert!(tally(&home, &["finalize"]).status.success());

    let conn = rusqlite::Connection::open(home.join("tally.db")).unwrap();
    conn.execute_batch("DROP TRIGGER ledger_no_update; DROP TRIGGER ledger_no_delete;").unwrap();
    let json: String = conn.query_row("SELECT entry_json FROM ledger WHERE seq = 1", [], |r| r.get(0)).unwrap();
    assert!(json.contains("\"verified_s\":120"), "{json}");
    let changed = json.replace("\"verified_s\":120", "\"verified_s\":121");
    conn.execute("UPDATE ledger SET entry_json = ?1 WHERE seq = 1", [changed]).unwrap();
    drop(conn);

    let o = tally(&home, &["verify"]);
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(out(&o), "FAIL at entry 0: bad_hash");
}

/// Home with the lunch scenario finalized under a project named owner/repo.
fn named_home(t: &Path) -> std::path::PathBuf {
    let home = t.join("home");
    let repo = t.join("repo");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    let o = tally(&home, &["project", "add", repo.to_str().unwrap(), "--name", "owner/repo"]);
    assert!(o.status.success(), "{}", err(&o));
    let id = out(&o);
    let mut evs = [ev("1", 0, prompt()), ev("2", 60, Kind::Idle { end_ts: 1_860_000 }), ev("3", 1900, prompt())];
    for e in &mut evs {
        e.project = Some(id.clone());
    }
    let file = t.join("events.jsonl");
    write_events(&file, &evs);
    assert!(tally(&home, &["ingest", file.to_str().unwrap(), "--watermark", "3000000"]).status.success());
    assert!(tally(&home, &["finalize"]).status.success());
    home
}

#[test]
fn publish_dry_run_prints_the_plan_and_changes_nothing() {
    let t = tempfile::tempdir().unwrap();
    let home = named_home(t.path());
    let before = out(&tally(&home, &["status"]));
    let o = tally(&home, &["publish", "--dry-run"]);
    assert!(o.status.success(), "{}", err(&o));
    let s = out(&o);
    assert!(s.contains("project owner/repo: 2 new entries, 2 total, 0.1 h"), "{s}");
    assert!(s.contains("\"message\":\"0.1 h\""), "{s}");
    assert!(s.contains("would write owner/repo/badge.json") && s.contains("dry run"), "{s}");
    assert_eq!(out(&tally(&home, &["status"])), before);
    assert_eq!(out(&tally(&home, &["publish", "--dry-run"])), s);
}

#[test]
fn publish_with_no_named_projects_says_so() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    write_events(&t.path().join("e.jsonl"), &[ev("1", 0, prompt())]);
    assert!(tally(&home, &["ingest", t.path().join("e.jsonl").to_str().unwrap()]).status.success());
    assert_eq!(out(&tally(&home, &["publish", "--dry-run"])), "no named projects; nothing to publish");
}

#[test]
fn publish_refuses_a_config_with_an_unknown_key() {
    let t = tempfile::tempdir().unwrap();
    let home = named_home(t.path());
    std::fs::write(home.join("config.toml"), "[publish]\nbogus = 1\n").unwrap();
    let o = tally(&home, &["publish", "--dry-run"]);
    assert!(!o.status.success());
    assert!(err(&o).contains("bogus"), "{}", err(&o));
}

#[test]
fn publish_pushes_once_then_has_nothing_to_push() {
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }
    let t = tempfile::tempdir().unwrap();
    let home = named_home(t.path());
    let bare = t.path().join("bare.git");
    assert!(Command::new("git").args(["init", "-q", "--bare", "-b", "tally-data"]).arg(&bare).output().unwrap().status.success());
    std::fs::write(home.join("config.toml"), format!("[publish]\nremote = '{}'\n", bare.display())).unwrap();
    let o = tally(&home, &["publish"]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(out(&o).contains("wrote owner/repo/badge.json") && out(&o).ends_with("pushed 2 entries"), "{}", out(&o));
    let o = tally(&home, &["publish"]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(out(&o).ends_with("nothing to push") && !out(&o).contains("wrote"), "{}", out(&o));
}

fn status_events(home: &Path) -> String {
    let s = out(&tally(home, &["status"]));
    s.lines().find(|l| l.starts_with("events:")).unwrap().to_string()
}

#[test]
fn hook_test_run_passes_the_exit_code_through_and_records_one_user_event() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    let bin = env!("CARGO_BIN_EXE_tally");
    let o = tally(&home, &["hook", "test-run", "--", bin, "--help"]);
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    assert_eq!(status_events(&home), "events: 1");

    let o = tally(&home, &["hook", "test-run", "--", bin, "no-such-subcommand"]);
    let code = o.status.code().unwrap();
    assert_ne!(code, 0);
    assert_eq!(code, Command::new(bin).arg("no-such-subcommand").output().unwrap().status.code().unwrap());
    assert_eq!(status_events(&home), "events: 2");
}

#[test]
fn hook_test_run_inside_claude_code_runs_the_program_but_records_nothing() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    let bin = env!("CARGO_BIN_EXE_tally");
    let o = tally(&home, &["hook", "test-run", "--", bin, "--help"]);
    assert_eq!(status_events(&home), "events: 1");
    assert_eq!(o.status.code(), Some(0));
    let o = Command::new(bin).arg("--home").arg(&home).args(["hook", "test-run", "--", bin, "no-such-subcommand"])
        .env("CLAUDECODE", "1").output().unwrap();
    assert_ne!(o.status.code(), Some(0));
    assert!(err(&o).contains("not recorded as a user action"), "{}", err(&o));
    assert_eq!(status_events(&home), "events: 1");
}

#[test]
fn hook_test_run_of_a_missing_program_exits_127_and_records_nothing() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    let o = tally(&home, &["hook", "test-run", "--", "tally-no-such-program-xyz"]);
    assert_eq!(o.status.code(), Some(127));
    assert!(err(&o).contains("tally-no-such-program-xyz"), "{}", err(&o));
    assert!(!home.join("tally.db").exists());
}

#[test]
fn verify_published_checks_files_without_creating_a_database() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    let claims: Vec<tally_scoring::Claim> = (0..2).map(|i| tally_scoring::Claim {
        action_id: format!("a{i}"), ts: i, project: Some("o/r".into()), kind: "prompt".into(),
        verified_s: 360, unverified_s: 0, reasons: vec![], features: Default::default(),
    }).collect();
    let good = serde_json::to_string(&tally_scoring::chain(&claims, tally_scoring::GENESIS_HASH)).unwrap();
    let (gf, bf) = (t.path().join("good.json"), t.path().join("bad.json"));
    std::fs::write(&gf, &good).unwrap();
    std::fs::write(&bf, good.replacen("\"verified_s\":360", "\"verified_s\":361", 1)).unwrap();

    let o = tally(&home, &["verify-published", gf.to_str().unwrap()]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(out(&o).contains("ok:") && out(&o).contains("verified_hours 0.2"), "{}", out(&o));

    let o = tally(&home, &["verify-published", gf.to_str().unwrap(), bf.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(1));
    assert!(out(&o).contains("FAIL at entry"), "{}", out(&o));
    assert!(out(&o).contains(&format!("{}: FAIL at entry", bf.display())), "{}", out(&o));
    assert!(!home.join("tally.db").exists());
}

#[test]
fn project_add_rejects_a_name_that_is_not_owner_slash_repo() {
    let t = tempfile::tempdir().unwrap();
    let o = tally(&t.path().join("home"), &["project", "add", t.path().to_str().unwrap(), "--name", "foo"]);
    assert!(!o.status.success());
    assert!(err(&o).contains("invalid project name"), "{}", err(&o));
}

#[test]
fn login_stores_the_token_and_never_prints_it() {
    use std::io::Write;
    use std::process::Stdio;
    let t = tempfile::tempdir().unwrap();
    let mut c = Command::new(env!("CARGO_BIN_EXE_tally"))
        .arg("--home").arg(t.path()).args(["login", "--server-id", "srv_1"])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    c.stdin.take().unwrap().write_all(b"tok_secret\n").unwrap();
    let o = c.wait_with_output().unwrap();
    assert!(o.status.success(), "{}", err(&o));
    assert_eq!(out(&o), "logged in");
    assert!(!out(&o).contains("tok_secret") && !err(&o).contains("tok_secret"));
    assert_eq!(std::fs::read_to_string(t.path().join("device_tokens")).unwrap(), "srv_1 tok_secret\n");

    let o = tally(t.path(), &["logout", "--server-id", "srv_1"]);
    assert_eq!(out(&o), "logged out");
    assert_eq!(out(&tally(t.path(), &["logout", "--server-id", "srv_1"])), "not logged in");
}

#[path = "support/fake_server.rs"]
mod fake_server;
use fake_server::FakeServer;

fn login(home: &Path, token: &str, extra: &[&str]) -> Output {
    use std::io::Write;
    use std::process::Stdio;
    let mut c = Command::new(env!("CARGO_BIN_EXE_tally"))
        .arg("--home").arg(home).args(["login", "--server-id", "srv_1"]).args(extra)
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    c.stdin.take().unwrap().write_all(format!("{token}\n").as_bytes()).unwrap();
    c.wait_with_output().unwrap()
}

/// `named_home` with owner/repo linked to srv_1 and `[sync]` pointing at `url`.
fn synced_home(t: &Path, url: &str) -> std::path::PathBuf {
    let home = named_home(t);
    let o = tally(&home, &["project", "link", t.join("repo").to_str().unwrap(), "--server-id", "srv_1"]);
    assert!(o.status.success(), "{}", err(&o));
    std::fs::write(home.join("config.toml"), format!("[sync]\nenabled = true\nserver = '{url}'\n")).unwrap();
    home
}

#[test]
fn sync_dry_run_reports_and_uploads_nothing() {
    let f = FakeServer::start("tok");
    let t = tempfile::tempdir().unwrap();
    let home = synced_home(t.path(), &f.url);
    let o = login(&home, "tok", &[]);
    assert!(o.status.success(), "{}", err(&o));
    let o = tally(&home, &["sync", "--dry-run"]);
    assert!(o.status.success(), "{}", err(&o));
    assert_eq!(out(&o), format!("owner/repo {}: 2 to upload", tally_scoring::RULESET_VERSION));
    assert!(f.requests().iter().all(|r| r.method != "POST"));
    assert!(f.chain("srv_1", fake_server::R).is_empty());

    let o = tally(&home, &["sync"]);
    assert!(o.status.success(), "{}", err(&o));
    assert_eq!(out(&o), format!("owner/repo {}: uploaded 2, server has 2", tally_scoring::RULESET_VERSION));
    assert_eq!(f.chain("srv_1", fake_server::R).len(), 2);
    assert_eq!(f.commits("srv_1").len(), 1);
    assert!(!out(&o).contains("tok") && !err(&o).contains("tok"));
}

#[test]
fn sync_without_login_says_so() {
    let f = FakeServer::start("tok");
    let t = tempfile::tempdir().unwrap();
    let home = synced_home(t.path(), &f.url);
    let o = tally(&home, &["sync"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(err(&o).contains("tally login --server-id srv_1"), "{}", err(&o));
}

#[test]
fn sync_without_a_server_names_the_setting() {
    let t = tempfile::tempdir().unwrap();
    let home = named_home(t.path());
    let o = tally(&home, &["sync"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(err(&o).contains("[sync] server"), "{}", err(&o));
}

#[test]
fn login_checks_the_token_against_a_configured_server() {
    let f = FakeServer::start("tok");
    let t = tempfile::tempdir().unwrap();
    let o = login(t.path(), "tok_wrong", &["--server", &f.url]);
    assert_eq!(o.status.code(), Some(1));
    assert!(err(&o).contains("device token rejected"), "{}", err(&o));
    assert!(!out(&o).contains("tok_wrong") && !err(&o).contains("tok_wrong"));
    // the token stays stored even when rejected
    assert_eq!(std::fs::read_to_string(t.path().join("device_tokens")).unwrap(), "srv_1 tok_wrong\n");
    let o = login(t.path(), "tok", &[]);
    assert!(o.status.success(), "{}", err(&o));
    assert_eq!(out(&o), "logged in");
    f.down(true);
    let o = login(t.path(), "tok", &[]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(err(&o).contains("warning"), "{}", err(&o));
}

#[test]
fn repair_on_a_clean_store_prints_ok_and_new_epoch_is_refused() {
    let t = tempfile::tempdir().unwrap();
    let home = named_home(t.path());
    let o = tally(&home, &["repair"]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(out(&o).starts_with("ok:"), "{}", out(&o));
    let o = tally(&home, &["repair", "--new-epoch", "--reason", "store-damaged"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(err(&o).contains("not available"), "{}", err(&o));
    let o = tally(&home, &["repair", "--new-epoch", "--reason", "my text"]);
    assert!(!o.status.success());
    assert!(err(&o).contains("invalid value"), "{}", err(&o));
}

#[test]
fn doctor_prints_a_line_per_configured_source() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    tally_daemon::store::Store::open(&home.join("tally.db")).unwrap();
    let o = tally(&home, &["doctor"]);
    assert!(o.status.success(), "{}", err(&o));
    assert_eq!(out(&o), "claude-code  no_logs  files=0  last=never  format=-  errors=0/0");
    let o = tally(&home, &["doctor", "--json"]);
    assert!(out(&o).contains("\"status\": \"no_logs\""), "{}", out(&o));
}

#[test]
fn heartbeat_config_prints_loopback_url_and_token_and_creates_token_file() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    let o = tally(&home, &["heartbeat-config"]);
    assert!(o.status.success(), "{}", err(&o));
    let s = out(&o);
    assert!(s.lines().any(|l| l == "api_url = http://127.0.0.1:25930/api"), "{s}");
    let key = s.lines().find_map(|l| l.strip_prefix("api_key = ")).expect("api_key line").to_string();
    assert!(key.len() >= 32, "{s}");
    let file = std::fs::read_to_string(home.join("heartbeat.token")).unwrap();
    assert_eq!(file.trim(), key);
    assert!(s.contains("heartbeat.token"), "{s}");
    assert_eq!(out(&tally(&home, &["heartbeat-config"])), s);
}

#[test]
fn doctor_lists_heartbeat_counters() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let mut store = tally_daemon::store::Store::open(&home.join("tally.db")).unwrap();
    let h = tally_daemon::source::Health {
        files_found: 1,
        records: 3,
        format: "wakatime".into(),
        counters: vec![("accepted".into(), 3), ("dropped_agent".into(), 2)],
        ..Default::default()
    };
    store.put_health("heartbeat", &h, 1).unwrap();
    drop(store);
    let o = tally(&home, &["doctor"]);
    assert!(o.status.success(), "{}", err(&o));
    let s = out(&o);
    let l = s.lines().find(|l| l.starts_with("heartbeat")).expect("heartbeat line");
    assert!(l.contains("accepted=3") && l.contains("dropped_agent=2"), "{s}");
    let o = tally(&home, &["doctor", "--json"]);
    assert!(out(&o).contains("\"dropped_agent\""), "{}", out(&o));
}

#[test]
fn rederive_runs_without_a_database() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("nope");
    let mut events = vec![ev("1", 0, prompt()), ev("2", 60, Kind::Idle { end_ts: 1_860_000 }), ev("3", 1900, prompt())];
    for e in &mut events {
        e.project = Some("P".into());
    }
    let input = |ruleset: &str| serde_json::json!({
        "ruleset": ruleset, "project": "P", "from_ms": 0, "to_ms": 10_000_000, "events": events, "review": {}
    });
    let f = t.path().join("in.json");
    std::fs::write(&f, input(tally_scoring::RULESET_VERSION).to_string()).unwrap();
    let o = tally(&home, &["rederive", f.to_str().unwrap()]);
    assert!(o.status.success(), "{}", err(&o));
    let s = out(&o);
    assert!(s.lines().any(|l| l == "verified_s: 660") && s.lines().any(|l| l == "unknown_ids: none"), "{s}");
    assert!(!home.exists());

    let gz = t.path().join("in.json.gz");
    let mut w = flate2::write::GzEncoder::new(std::fs::File::create(&gz).unwrap(), flate2::Compression::default());
    std::io::Write::write_all(&mut w, input(tally_scoring::RULESET_VERSION).to_string().as_bytes()).unwrap();
    w.finish().unwrap();
    let o = tally(&home, &["rederive", gz.to_str().unwrap(), "--json"]);
    assert!(o.status.success(), "{}", err(&o));
    assert_eq!(serde_json::from_str::<serde_json::Value>(&out(&o)).unwrap()["verified_s"], 660);

    std::fs::write(&f, input("2026.11-0").to_string()).unwrap();
    let o = tally(&home, &["rederive", f.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(2));
    assert!(err(&o).contains("ruleset mismatch"), "{}", err(&o));
}

// ---- tally submit ----

/// Stub server: records every request as (head, body) and replies 201 to all.
fn stub() -> (String, std::sync::mpsc::Receiver<(String, Vec<u8>)>) {
    use std::io::{Read, Write};
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for s in l.incoming() {
            let mut s = s.unwrap();
            let mut buf = vec![];
            let mut b = [0u8; 4096];
            let (head, body_at) = loop {
                let n = s.read(&mut b).unwrap();
                if n == 0 {
                    break (String::new(), 0);
                }
                buf.extend_from_slice(&b[..n]);
                if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    break (String::from_utf8_lossy(&buf[..i]).to_string(), i + 4);
                }
            };
            let len = head.to_lowercase().lines().find_map(|l| l.strip_prefix("content-length: ").and_then(|v| v.trim().parse::<usize>().ok())).unwrap_or(0);
            while buf.len() < body_at + len {
                let n = s.read(&mut b).unwrap();
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&b[..n]);
            }
            let _ = s.write_all(b"HTTP/1.1 201 Created\r\ncontent-type: application/json\r\ncontent-length: 31\r\nconnection: close\r\n\r\n{\"id\":\"s1\",\"status\":\"uploaded\"}");
            let _ = tx.send((head, buf[body_at..].to_vec()));
        }
    });
    (url, rx)
}

/// A home with project o/one linked to srv_1, a logged-in token, and finalized prompt events.
fn submit_home(t: &Path, server: &str) -> PathBuf {
    let home = t.join("home");
    let repo = t.join("repo");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    let id = out(&tally(&home, &["project", "add", repo.to_str().unwrap(), "--name", "o/one"]));
    assert!(tally(&home, &["project", "link", repo.to_str().unwrap(), "--server-id", "srv_1"]).status.success());
    std::fs::write(home.join("config.toml"), format!("[sync]\nenabled = true\nserver = \"{server}\"\n")).unwrap();
    std::fs::write(home.join("device_tokens"), "srv_1 tok\n").unwrap();
    let evs: Vec<Event> = (0..5)
        .map(|k| {
            let mut e = ev(&format!("p{k}"), 1000 + k * 60, prompt());
            e.project = Some(id.clone());
            e
        })
        .collect();
    let f = t.join("events.jsonl");
    write_events(&f, &evs);
    assert!(tally(&home, &["ingest", f.to_str().unwrap(), "--watermark", "100000000"]).status.success());
    assert!(tally(&home, &["finalize"]).status.success());
    home
}

fn submit(home: &Path, stdin: &str) -> Output {
    use std::io::Write;
    let mut c = Command::new(env!("CARGO_BIN_EXE_tally"))
        .arg("--home").arg(home)
        .args(["submit", "o/one", "--from", "0", "--to", "1000000"])
        .env_remove("CLAUDECODE")
        .stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped())
        .spawn().unwrap();
    c.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    c.wait_with_output().unwrap()
}

fn submissions_empty(home: &Path) -> bool {
    std::fs::read_dir(home.join("submissions")).map_or(true, |d| d.flatten().all(|e| std::fs::read_dir(e.path()).map_or(true, |mut f| f.next().is_none())))
}

#[test]
fn submit_uploads_only_after_yes() {
    use std::io::Read;
    let t = tempfile::tempdir().unwrap();
    let (url, rx) = stub();
    let home = submit_home(t.path(), &url);
    let o = submit(&home, "yes\n");
    assert!(o.status.success(), "{}", err(&o));
    assert!(out(&o).contains("submitted s1"), "{}", out(&o));
    let reqs: Vec<_> = rx.try_iter().filter(|(h, _)| h.starts_with("POST ")).collect();
    assert_eq!(reqs.len(), 1);
    let (head, body) = &reqs[0];
    assert!(head.starts_with("POST /v1/projects/srv_1/submissions "), "{head}");
    assert!(!head.contains("o/one") && !head.contains("o%2F"), "{head}");
    assert!(head.to_lowercase().contains("content-encoding: gzip") && head.contains("tok"), "{head}");
    let mut json = String::new();
    flate2::read::GzDecoder::new(&body[..]).read_to_string(&mut json).unwrap();
    let b: tally_daemon::submit::Bundle = serde_json::from_str(&json).unwrap();
    assert_eq!((b.format.as_str(), b.project.as_str()), ("tally-bundle/1", "o/one"));
    assert!(submissions_empty(&home));
}

#[test]
fn submit_declined_uploads_nothing() {
    for answer in ["no\n", "Yes\n", ""] {
        let t = tempfile::tempdir().unwrap();
        let (url, rx) = stub();
        let home = submit_home(t.path(), &url);
        let o = submit(&home, answer);
        assert!(o.status.success(), "{}", err(&o));
        assert!(out(&o).contains("not submitted"), "{}", out(&o));
        assert_eq!(rx.try_iter().filter(|(h, _)| h.starts_with("POST ")).count(), 0);
        assert!(submissions_empty(&home));
    }
}
