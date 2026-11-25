//! The load generator's output must be something the real Claude Code adapter ingests cleanly.
use std::path::PathBuf;
use std::process::Command;
use tally_evidence::load::{write_session_log, LoadPlan, RealClock};

/// Builds `tally-daemon` (a bin of another package is not built by `cargo test -p tally-evidence`) and asks cargo where
/// the `tally` executable is; the target directory layout differs between setups, so it is not guessed.
fn tally_bin() -> PathBuf {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let out = Command::new(cargo).args(["build", "-q", "-p", "tally-daemon", "--bin", "tally", "--message-format=json"]).output().unwrap();
    assert!(out.status.success(), "building tally-daemon: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|v| v["reason"] == "compiler-artifact" && v["target"]["name"] == "tally")
        .find_map(|v| v["executable"].as_str().map(PathBuf::from))
        .expect("cargo reported no tally executable")
}

#[test]
fn an_hour_of_load_is_ingested_by_the_real_adapter_without_warnings() {
    let t = tempfile::tempdir().unwrap();
    let (home, logs, repo) = (t.path().join("home"), t.path().join("logs"), t.path().join("repo"));
    for d in [&home, &logs, &repo] {
        std::fs::create_dir_all(d).unwrap();
    }
    assert!(Command::new("git").arg("init").arg("-q").arg(&repo).status().unwrap().success(), "git init");
    std::fs::write(home.join("config.toml"), format!("[claude_code]\nprojects_dir = '{}'\n[presence]\nenabled = false\nrequire = false\n", logs.display())).unwrap();

    // The adapter does live capture: the first poll of a home records where each file ends and reads no history.
    // So initialise the home first, then write the load (the resource harness likewise starts the daemon before the writer).
    let tally = tally_bin();
    let first = Command::new(&tally).arg("--home").arg(&home).args(["run", "--once"]).output().unwrap();
    assert!(first.status.success(), "{}", String::from_utf8_lossy(&first.stderr));

    let mut plan = LoadPlan::default_for(repo);
    plan.hours = 1;
    plan.speedup = 1000; // 40 prompts, about 4 s of wall clock
    write_session_log(&logs, &plan, &mut RealClock).unwrap();

    let out = Command::new(&tally).arg("--home").arg(&home).args(["run", "--once"]).output().unwrap();
    let (so, se) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{so}\n{se}");
    let added: u32 = so.split("added ").nth(1).and_then(|r| r.split(',').next()).and_then(|n| n.trim().parse().ok()).unwrap_or(0);
    assert!(added > 0, "no events ingested: {so}\n{se}");
    assert!(!se.to_lowercase().contains("warn") && !se.to_lowercase().contains("error"), "adapter complained: {se}");
}
