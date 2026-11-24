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

#[test]
fn stop_pauses_and_ensure_respects_it() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let held = lock(&home);
    // Stands in for the daemon: lets go of run.lock once `paused` appears.
    let h = home.clone();
    let releaser = std::thread::spawn(move || {
        let t0 = std::time::Instant::now();
        while !h.join("paused").exists() && t0.elapsed().as_secs() < 15 {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        drop(held);
    });
    let o = tally(&home, &["stop"]);
    releaser.join().unwrap();
    assert!(o.status.success());
    assert_eq!(String::from_utf8_lossy(&o.stdout).trim(), "stopped");
    let r = ensure(&home, &opts(), &|_| panic!("spawn must not be called"));
    assert!(!r.started && !r.first_run);
}

#[test]
fn start_clears_the_pause() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    let o = tally(&home, &["stop"]);
    assert!(o.status.success());
    assert_eq!(String::from_utf8_lossy(&o.stdout).trim(), "not running");
    assert!(home.join("paused").exists());
    // A held lock keeps `start` from spawning a real daemon.
    let held = lock(&home);
    let o = tally(&home, &["start"]);
    assert!(o.status.success());
    assert_eq!(String::from_utf8_lossy(&o.stdout).trim(), "already running");
    assert!(!home.join("paused").exists());
    drop(held);
    let n = Cell::new(0);
    let r = ensure(&home, &opts(), &|_| {
        n.set(n.get() + 1);
        Ok(())
    });
    assert!(r.started);
    assert_eq!(n.get(), 1);
}

/// Kills the daemon it is given the home of, on drop (so a failed assert cannot leak it).
struct Reap<'a>(&'a Path);
impl Drop for Reap<'_> {
    fn drop(&mut self) {
        kill_daemons(self.0);
    }
}

#[test]
fn a_fresh_home_serves_heartbeats_after_ensure() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    // Not the default port: a maintainer's own daemon (or a parallel test's) may hold 25930.
    // `enabled` is left at its default, which is what this test is about.
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("config.toml"), format!("[heartbeat]\nport = {port}\n")).unwrap();
    let cfg = String::from_utf8(tally(&home, &["heartbeat-config"]).stdout).unwrap();
    let get = |k: &str| cfg.lines().find_map(|l| l.strip_prefix(k)).unwrap().trim().to_string();
    let (url, key) = (get("api_url = "), get("api_key = "));
    assert!(url.contains(&format!(":{port}/")), "{url}");

    assert!(tally(&home, &["ensure"]).status.success());
    let _reap = Reap(&home);
    let url = format!("{url}/v1/users/current/heartbeats");
    let body = r#"{"entity":"C:/p/a.rs","type":"file","time":1763800000.5,"is_write":true}"#;
    let code = |auth: Option<&str>| {
        let mut r = ureq::post(&url);
        if let Some(a) = auth {
            r = r.set("Authorization", a);
        }
        match r.set("Content-Type", "application/json").send_string(body) {
            Ok(r) => Some(r.status()),
            Err(ureq::Error::Status(c, _)) => Some(c),
            Err(_) => None,
        }
    };
    let start = std::time::Instant::now();
    let mut unauth = code(None);
    while unauth.is_none() && start.elapsed().as_secs() < 5 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        unauth = code(None);
    }
    assert_eq!(unauth, Some(401), "heartbeat server did not come up on port {port}");
    use base64::Engine;
    let basic = format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(&key));
    assert_eq!(code(Some(&basic)), Some(201));
}

#[test]
fn status_brief_is_one_line() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    let file = t.path().join("events.jsonl");
    // One prompt: a single 120 s claim.
    let ev = |id: &str, ts: i64| {
        format!(r#"{{"v":1,"id":"{id}","ts":{ts},"source":"test","project":null,"session":null,"kind":"prompt","chars":200,"relevance_pm":600,"novelty_bytes":100}}"#)
    };
    std::fs::write(&file, format!("{}\n", ev("1", 0))).unwrap();
    assert!(tally(&home, &["ingest", file.to_str().unwrap(), "--watermark", "3000000"]).status.success());
    assert!(tally(&home, &["finalize"]).status.success());
    let o = tally(&home, &["status", "--brief"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let s = String::from_utf8(o.stdout).unwrap();
    assert_eq!(s.lines().count(), 1, "{s}");
    assert!(s.contains("0h 2m verified") && s.contains("daemon stopped"), "{s}");
}
