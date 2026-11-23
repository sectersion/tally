use std::path::Path;
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
    assert!(!home.join("tally.db").exists());
}
