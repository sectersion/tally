use std::path::Path;
use std::process::{Command, Output};

fn tally(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tally"))
        .arg("--home")
        .arg(home)
        .args(args)
        .env_remove("CLAUDECODE")
        .output()
        .unwrap()
}
fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).to_string()
}

#[test]
fn second_run_on_one_home_is_refused() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    let mut first = Command::new(env!("CARGO_BIN_EXE_tally"))
        .arg("--home")
        .arg(&home)
        .args(["run", "--interval-secs", "3600"])
        .env_remove("CLAUDECODE")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(std::time::Duration::from_secs(2));
    let o = tally(&home, &["run", "--interval-secs", "3600"]);
    first.kill().unwrap();
    first.wait().unwrap();
    assert!(!o.status.success());
    assert!(err(&o).contains("already"), "{}", err(&o));
}

#[test]
fn project_add_warns_when_name_changes() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    let repo = t.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    assert!(Command::new("git")
        .arg("init")
        .arg(&repo)
        .output()
        .unwrap()
        .status
        .success());
    let r = repo.to_str().unwrap();
    assert!(tally(&home, &["project", "add", r, "--name", "a/one"])
        .status
        .success());
    let o = tally(&home, &["project", "add", r, "--name", "a/two"]);
    assert!(o.status.success());
    assert!(err(&o).contains("renamed"), "{}", err(&o));
}
