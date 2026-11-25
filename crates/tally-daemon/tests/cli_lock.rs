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
    let run = |piped: bool| {
        let mut c = Command::new(env!("CARGO_BIN_EXE_tally"));
        c.arg("--home")
            .arg(&home)
            .args(["run", "--interval-secs", "3600"])
            .env_remove("CLAUDECODE");
        c.stdout(std::process::Stdio::null());
        c.stderr(if piped {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        });
        c.spawn().unwrap()
    };
    let mut first = run(false);
    std::thread::sleep(std::time::Duration::from_secs(2));
    let mut second = run(true);
    // Without the lock the second daemon never exits; bound the wait so that fails instead of hanging.
    let mut status = None;
    for _ in 0..50 {
        if let Some(s) = second.try_wait().unwrap() {
            status = Some(s);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    if status.is_none() {
        second.kill().unwrap();
    }
    first.kill().unwrap();
    first.wait().unwrap();
    let o = second.wait_with_output().unwrap();
    assert!(
        status.is_some_and(|s| !s.success()),
        "second run was not refused"
    );
    // 75 (EX_TEMPFAIL) lets the service units skip their restart loop: systemd RestartPreventExitStatus=75
    assert_eq!(status.and_then(|s| s.code()), Some(75), "a held lock must exit 75");
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
