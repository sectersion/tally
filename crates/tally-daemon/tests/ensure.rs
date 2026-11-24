use std::cell::Cell;
use std::path::Path;
use std::process::{Command, Output};
use tally_daemon::ensure::{daemon_running, ensure, EnsureOpts};
use tally_daemon::store::Store;

fn tally(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tally")).arg("--home").arg(home).args(args).env_remove("CLAUDECODE").output().unwrap()
}
fn opts() -> EnsureOpts {
    EnsureOpts { client: Some("claude-code".into()), register_cwd: None }
}
fn lock(home: &Path) -> std::fs::File {
    let f = std::fs::OpenOptions::new().create(true).write(true).truncate(false).open(home.join("run.lock")).unwrap();
    f.try_lock().unwrap();
    f
}

#[test]
fn first_run_records_consent_and_starts_the_daemon() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    let n = Cell::new(0);
    let r = ensure(&home, &opts(), &|_| {
        n.set(n.get() + 1);
        Ok(())
    });
    assert!(r.first_run && r.started);
    assert!(r.notice.is_some());
    assert_eq!(n.get(), 1);
    let c: serde_json::Value = serde_json::from_slice(&std::fs::read(home.join("consent.json")).unwrap()).unwrap();
    assert_eq!(c["client"], "claude-code");
}

#[test]
fn second_run_changes_nothing() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    ensure(&home, &opts(), &|_| Ok(()));
    let before = std::fs::read(home.join("consent.json")).unwrap();
    let _held = lock(&home);
    let r = ensure(&home, &opts(), &|_| panic!("spawn must not be called"));
    assert!(!r.first_run && !r.started && r.notice.is_none());
    assert_eq!(std::fs::read(home.join("consent.json")).unwrap(), before);
}

#[test]
fn a_failing_spawn_is_logged_not_raised() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    let r = ensure(&home, &opts(), &|_| Err(std::io::Error::other("boom-spawn")));
    assert!(!r.started);
    assert!(std::fs::read_to_string(home.join("ensure.log")).unwrap().contains("boom-spawn"));

    let file = t.path().join("afile");
    std::fs::write(&file, "x").unwrap();
    let o = tally(&file.join("sub"), &["ensure"]);
    assert!(o.status.success());
    assert!(o.stderr.is_empty(), "{}", String::from_utf8_lossy(&o.stderr));
}

#[test]
fn register_cwd_registers_an_unnamed_project_once() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    let repo = t.path().join("my répo");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    let o = EnsureOpts { client: None, register_cwd: Some(repo) };
    ensure(&home, &o, &|_| Ok(()));
    ensure(&home, &o, &|_| Ok(()));
    let rows = Store::open(&home.join("tally.db")).unwrap().projects().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].public_name, None);
}

#[test]
fn hook_json_is_printed_only_on_the_first_run() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    // A held lock keeps the CLI from spawning a real daemon.
    std::fs::create_dir_all(&home).unwrap();
    let _held = lock(&home);
    let o = tally(&home, &["ensure", "--hook-json"]);
    assert!(o.status.success());
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert!(!v["systemMessage"].as_str().unwrap().is_empty());
    let o = tally(&home, &["ensure", "--hook-json"]);
    assert!(o.status.success() && o.stdout.is_empty());
}

#[test]
fn concurrent_ensures_leave_one_daemon() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    let kids: Vec<_> = (0..4)
        .map(|_| Command::new(env!("CARGO_BIN_EXE_tally")).arg("--home").arg(&home).arg("ensure").spawn().unwrap())
        .collect();
    for mut k in kids {
        assert!(k.wait().unwrap().success());
    }
    let start = std::time::Instant::now();
    while !daemon_running(&home) && start.elapsed().as_secs() < 5 {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let running = daemon_running(&home);
    let second = std::fs::OpenOptions::new().write(true).open(home.join("run.lock")).unwrap().try_lock().is_err();
    // Stop the daemon: it has no pid file, so find it through the OS.
    kill_daemons(&home);
    assert!(running, "no daemon holds run.lock");
    assert!(second);
}

fn kill_daemons(home: &Path) {
    let h = home.to_string_lossy().to_string();
    if cfg!(windows) {
        let ps = format!(
            "Get-CimInstance Win32_Process -Filter \"Name='tally.exe'\" | Where-Object {{ $_.CommandLine -like '*run --home*{}*' }} | ForEach-Object {{ Stop-Process -Id $_.ProcessId -Force }}",
            h.replace('\'', "''")
        );
        let _ = Command::new("powershell").args(["-NoProfile", "-Command", &ps]).output();
    } else {
        let _ = Command::new("pkill").args(["-f", &format!("run --home {h}")]).output();
    }
    let start = std::time::Instant::now();
    while daemon_running(home) && start.elapsed().as_secs() < 5 {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

#[test]
fn unwritable_consent_starts_and_registers_nothing() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    std::fs::create_dir_all(home.join("consent.json")).unwrap();
    let repo = t.path().join("repo");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    let o = EnsureOpts { client: None, register_cwd: Some(repo) };
    let r = ensure(&home, &o, &|_| panic!("spawn must not be called"));
    assert!(!r.first_run && !r.started);
    assert!(std::fs::read_to_string(home.join("ensure.log")).unwrap().contains("consent.json"));
    assert!(!home.join("tally.db").exists());
}

#[test]
fn concurrent_first_runs_report_first_run_once() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let firsts: usize = std::thread::scope(|s| {
        let hs: Vec<_> = (0..8).map(|_| s.spawn(|| ensure(&home, &opts(), &|_| Ok(())).first_run)).collect();
        hs.into_iter().map(|h| usize::from(h.join().unwrap())).sum()
    });
    assert_eq!(firsts, 1);
}

#[test]
fn hook_json_returns_while_the_daemon_keeps_running() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    // output() reads stdout through a pipe until EOF: it hangs if the daemon inherited the pipe.
    let (tx, rx) = std::sync::mpsc::channel();
    let h = home.clone();
    std::thread::spawn(move || {
        let _ = tx.send(tally(&h, &["ensure", "--hook-json"]));
    });
    let res = rx.recv_timeout(std::time::Duration::from_secs(8));
    let start = std::time::Instant::now();
    while res.is_ok() && !daemon_running(&home) && start.elapsed().as_secs() < 5 {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let up = daemon_running(&home);
    kill_daemons(&home);
    let o = res.expect("tally ensure did not return: the daemon holds the host's pipe");
    assert!(o.status.success() && !o.stdout.is_empty());
    assert!(up);
}
