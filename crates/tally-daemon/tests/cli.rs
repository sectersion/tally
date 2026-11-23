use std::path::Path;
use std::process::{Command, Output};
use tally_scoring::{Event, Kind};

fn tally(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tally")).arg("--home").arg(home).args(args).output().unwrap()
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
    assert_eq!(out(&o), "appended 2, final 2, provisional 0");

    let o = tally(&home, &["status"]);
    let s = out(&o);
    assert!(s.contains("final_claims: 2"), "{s}");
    assert!(s.contains("verified_s: 660"), "{s}");

    let o = tally(&home, &["verify"]);
    assert!(o.status.success());
    assert_eq!(out(&o), "ok: 2 entries");

    assert_eq!(out(&tally(&home, &["ingest", f])), "added 0, duplicates 3");

    let late = t.path().join("late.jsonl");
    write_events(&late, &[ev("4", 1000, prompt())]);
    let o = tally(&home, &["ingest", late.to_str().unwrap()]);
    assert!(!o.status.success());
    assert!(err(&o).contains("late event"), "{}", err(&o));

    assert_eq!(out(&tally(&home, &["finalize"])), "appended 0, final 2, provisional 0");
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
fn a_bad_event_line_names_its_line_number() {
    let t = tempfile::tempdir().unwrap();
    let file = t.path().join("bad.jsonl");
    let good = serde_json::to_string(&ev("1", 0, prompt())).unwrap();
    std::fs::write(&file, format!("{good}\nnot json\n")).unwrap();
    let o = tally(&t.path().join("home"), &["ingest", file.to_str().unwrap()]);
    assert!(!o.status.success());
    assert!(err(&o).contains("line 2"), "{}", err(&o));
}
