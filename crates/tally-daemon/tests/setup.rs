use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tally_daemon::ensure::daemon_running;
use tally_daemon::setup::wakatime::{write_cfg, CfgResult};
use tally_daemon::setup::{registry, run, Detection, Env, Os, Outcome, SetupOpts};

struct Fake {
    os: Os,
    home: PathBuf,
    path: HashMap<String, PathBuf>,
    files: RefCell<HashSet<PathBuf>>,
    vars: HashMap<String, String>,
    log: RefCell<Vec<Vec<String>>>,
    reply: Box<dyn Fn(&str) -> String>,
}

fn fake(os: Os, home: &str) -> Fake {
    Fake { os, home: PathBuf::from(home), path: HashMap::new(), files: RefCell::default(), vars: HashMap::new(), log: RefCell::default(), reply: Box::new(|_| String::new()) }
}

impl Env for Fake {
    fn os(&self) -> Os {
        self.os
    }
    fn home(&self) -> PathBuf {
        self.home.clone()
    }
    fn which(&self, name: &str) -> Option<PathBuf> {
        self.path.get(name).cloned()
    }
    fn exists(&self, p: &Path) -> bool {
        self.files.borrow().contains(p)
    }
    fn var(&self, k: &str) -> Option<String> {
        self.vars.get(k).cloned()
    }
    fn run(&self, program: &Path, args: &[String]) -> std::io::Result<Output> {
        let mut argv = vec![program.to_string_lossy().to_string()];
        argv.extend(args.iter().cloned());
        let out = (self.reply)(&argv.join(" "));
        self.log.borrow_mut().push(argv);
        #[cfg(windows)]
        let status = std::os::windows::process::ExitStatusExt::from_raw(0);
        #[cfg(unix)]
        let status = std::os::unix::process::ExitStatusExt::from_raw(0);
        Ok(Output { status, stdout: out.into_bytes(), stderr: vec![] })
    }
}

fn opts(extras: &Path) -> SetupOpts {
    SetupOpts { yes: true, extras_dir: extras.to_path_buf(), ..Default::default() }
}

fn go(e: &Fake, o: &SetupOpts) -> tally_daemon::setup::Report {
    run(e, o, &mut std::io::empty(), &mut std::io::sink()).unwrap()
}

fn outcome(r: &tally_daemon::setup::Report, name: &str) -> Result<Outcome, String> {
    r.rows.iter().find(|x| x.name == name).unwrap().outcome.clone().unwrap()
}

fn detect(e: &Fake, name: &str) -> Detection {
    registry(false).iter().find(|i| i.name() == name).unwrap().detect(e)
}

fn linux_with_extras() -> (Fake, PathBuf) {
    let mut e = fake(Os::Linux, "/home/jo");
    let extras = PathBuf::from("/opt/tally/extras");
    e.files.get_mut().insert(extras.join("tally-vscode.vsix"));
    e.path.insert("code".into(), "/usr/bin/code".into());
    (e, extras)
}

#[test]
fn finds_an_editor_on_path_by_fallback_path_and_never_by_config_alone() {
    let mut e = fake(Os::Linux, "/home/jo");
    e.path.insert("code".into(), "/usr/bin/code".into());
    e.files.get_mut().insert(Path::new("/snap/bin").join("cursor"));
    e.files.get_mut().insert(e.home.join(".windsurf"));
    assert_eq!(detect(&e, "VS Code"), Detection::Found { cli: "/usr/bin/code".into() });
    assert_eq!(detect(&e, "Cursor"), Detection::Found { cli: Path::new("/snap/bin").join("cursor") });
    assert!(matches!(detect(&e, "Windsurf"), Detection::ConfigOnly { .. }));
    assert_eq!(detect(&e, "Trae"), Detection::NotFound);
    let r = go(&e, &opts(Path::new("/x")));
    assert!(e.log.borrow().iter().all(|c| !c.join(" ").contains("windsurf")), "{:?}", e.log.borrow());
    assert!(r.rows.iter().find(|x| x.name == "Windsurf").unwrap().outcome.is_none());
    // Windows: the known install folders.
    let mut w = fake(Os::Windows, "C:\\Users\\Jo Smith");
    w.vars.insert("LOCALAPPDATA".into(), "C:\\Users\\Jo Smith\\AppData\\Local".into());
    let p = Path::new("C:\\Users\\Jo Smith\\AppData\\Local").join("Programs").join("cursor").join("bin").join("cursor.cmd");
    w.files.get_mut().insert(p.clone());
    assert_eq!(detect(&w, "Cursor"), Detection::Found { cli: p });
}

#[test]
fn installs_the_vsix_into_every_vscode_family_editor_found() {
    let (mut e, extras) = linux_with_extras();
    e.path.insert("cursor".into(), "/usr/bin/cursor".into());
    go(&e, &opts(&extras));
    let vsix = extras.join("tally-vscode.vsix").to_string_lossy().to_string();
    let installs: Vec<_> = e.log.borrow().iter().filter(|c| c.iter().any(|a| a == "--install-extension")).cloned().collect();
    assert_eq!(installs.len(), 2, "{installs:?}");
    for c in &installs {
        assert_eq!(c[1..], ["--install-extension".to_string(), vsix.clone(), "--force".to_string()]);
    }

    // Windows: through `cmd /S /C`, paths with spaces quoted inside the one command line.
    let mut w = fake(Os::Windows, "C:\\Users\\Jo Smith");
    let code = "C:\\Program Files\\Microsoft VS Code\\bin\\code.cmd";
    let cursor = "C:\\Users\\Jo Smith\\AppData\\Local\\Programs\\cursor\\bin\\cursor.cmd";
    w.path.insert("code".into(), code.into());
    w.path.insert("cursor".into(), cursor.into());
    let extras = Path::new("C:\\Users\\Jo Smith\\tally").join("extras");
    w.files.get_mut().insert(extras.join("tally-vscode.vsix"));
    go(&w, &opts(&extras));
    let vsix = extras.join("tally-vscode.vsix").display().to_string();
    let log = w.log.borrow();
    let installs: Vec<_> = log.iter().filter(|c| c[3].contains("--install-extension")).collect();
    println!("{installs:#?}");
    assert_eq!(installs.len(), 2);
    assert_eq!(installs[0][..3], ["cmd", "/S", "/C"]);
    assert_eq!(installs[0][3], format!("\"{code}\" --install-extension \"{vsix}\" --force"));
    assert_eq!(installs[1][3], format!("\"{cursor}\" --install-extension \"{vsix}\" --force"));
}

#[test]
fn dry_run_runs_nothing_and_writes_nothing() {
    let (e, extras) = linux_with_extras();
    let before = e.files.borrow().clone();
    let r = go(&e, &SetupOpts { dry_run: true, ..opts(&extras) });
    assert!(e.log.borrow().is_empty());
    assert_eq!(*e.files.borrow(), before);
    assert!(matches!(outcome(&r, "VS Code"), Ok(Outcome::Skipped(m)) if m.starts_with("would install")));
}

#[test]
fn second_run_changes_nothing() {
    let (mut e, extras) = linux_with_extras();
    e.reply = Box::new(|c| if c.ends_with("--list-extensions") { "ms-python.python\nsectersion.tally\n".into() } else { String::new() });
    let r = go(&e, &opts(&extras));
    assert_eq!(outcome(&r, "VS Code"), Ok(Outcome::AlreadyInstalled));
    assert!(e.log.borrow().iter().all(|c| !c.iter().any(|a| a == "--install-extension")));
}

#[test]
fn claude_code_adds_the_marketplace_then_installs_once() {
    let mut e = fake(Os::Linux, "/home/jo");
    e.path.insert("claude".into(), "claude".into());
    e.reply = Box::new(|_| "[]".into());
    let r = go(&e, &opts(Path::new("/x")));
    assert_eq!(outcome(&r, "Claude Code"), Ok(Outcome::Installed));
    let cmds: Vec<String> = e.log.borrow().iter().map(|c| c.join(" ")).filter(|c| !c.ends_with("list --json")).collect();
    assert_eq!(cmds, ["claude plugin marketplace add sectersion/tally", "claude plugin install tally@tally"]);

    e.log.borrow_mut().clear();
    e.reply = Box::new(|_| r#"[{"id":"tally@tally","version":"0.1.0"}]"#.into());
    let r = go(&e, &opts(Path::new("/x")));
    assert_eq!(outcome(&r, "Claude Code"), Ok(Outcome::AlreadyInstalled));
    assert!(e.log.borrow().iter().all(|c| c.ends_with(&["list".to_string(), "--json".to_string()])));
}

#[test]
fn a_missing_vsix_fails_that_editor_only() {
    let (mut e, _) = linux_with_extras();
    e.path.insert("claude".into(), "claude".into());
    e.reply = Box::new(|_| "[]".into());
    let r = go(&e, &opts(Path::new("/nowhere")));
    assert!(outcome(&r, "VS Code").unwrap_err().contains("tally-vscode.vsix"));
    assert_eq!(outcome(&r, "Claude Code"), Ok(Outcome::Installed));
}

#[test]
fn wakatime_cfg_is_never_overwritten_without_force() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join(".wakatime.cfg");
    let (url, key) = ("http://127.0.0.1:25930/api", "11111111-2222-3333-4444-555555555555");
    let old = "[settings]\napi_url = https://wakatime.com/api\ndebug = true\n\n[projectmap]\nfoo = bar\n";
    std::fs::write(&p, old).unwrap();
    assert_eq!(write_cfg(&p, url, key, false), CfgResult::Conflict { existing_url: "https://wakatime.com/api".into() });
    assert_eq!(std::fs::read_to_string(&p).unwrap(), old);

    assert_eq!(write_cfg(&p, url, key, true), CfgResult::BackedUpAndWritten);
    assert_eq!(std::fs::read_to_string(t.path().join(".wakatime.cfg.tally-bak")).unwrap(), old);
    let new = std::fs::read_to_string(&p).unwrap();
    assert!(new.contains(&format!("api_url = {url}")) && new.contains(&format!("api_key = {key}")));
    assert!(new.contains("debug = true") && new.contains("[projectmap]\nfoo = bar") && !new.contains("wakatime.com"));
    assert_eq!(write_cfg(&p, url, key, false), CfgResult::AlreadyOurs);

    let fresh = t.path().join("fresh.cfg");
    assert_eq!(write_cfg(&fresh, url, key, false), CfgResult::Written);
    assert_eq!(write_cfg(&fresh, url, key, false), CfgResult::AlreadyOurs);
}

fn tally(home: &Path, args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_tally"));
    c.arg("--home").arg(home).args(args).env("TALLY_NO_TTY", "1").env_remove("CLAUDECODE").stdin(std::process::Stdio::null());
    c
}

#[test]
fn no_terminal_and_no_yes_changes_nothing() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    let o = tally(&home, &["setup", "--only", "jetbrains"]).output().unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("--yes"));
    assert!(!home.exists());
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
    let t0 = std::time::Instant::now();
    while daemon_running(home) && t0.elapsed().as_secs() < 5 {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

#[test]
fn setup_ends_with_a_running_daemon_and_a_201_test_heartbeat() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    std::fs::write(home.join("config.toml"), format!("[heartbeat]\nport = {port}\n")).unwrap();
    let o = tally(&home, &["setup", "--yes", "--only", "jetbrains"]).output().unwrap();
    let up = daemon_running(&home);
    kill_daemons(&home);
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(o.status.success(), "{out} {}", String::from_utf8_lossy(&o.stderr));
    assert!(up, "{out}");
    assert!(out.contains("test heartbeat: 201"), "{out}");
}

#[test]
fn remove_uninstalls_and_leaves_the_home_alone() {
    let (mut e, extras) = linux_with_extras();
    e.path.insert("claude".into(), "claude".into());
    e.reply = Box::new(|c| match c {
        c if c.ends_with("--list-extensions") => "sectersion.tally\n".into(),
        c if c.ends_with("list --json") => r#"[{"id":"tally@tally"}]"#.into(),
        c if c.ends_with("marketplace list") => "tally (github: sectersion/tally)\n".into(),
        _ => String::new(),
    });
    go(&e, &SetupOpts { remove: true, ..opts(&extras) });
    let cmds: Vec<String> = e.log.borrow().iter().map(|c| c.join(" ")).collect();
    for want in ["/usr/bin/code --uninstall-extension sectersion.tally", "claude plugin uninstall tally@tally", "claude plugin marketplace remove tally"] {
        assert!(cmds.iter().any(|c| c == want), "{want} not in {cmds:?}");
    }
    assert!(cmds.iter().all(|c| !c.contains("--install-extension") && !c.contains("plugin install")));

    // The real binary with nothing selected: the data directory is byte-identical.
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("tally.db"), b"not really a database").unwrap();
    let o = tally(&home, &["setup", "--remove", "--yes", "--only", "jetbrains"]).output().unwrap();
    assert!(o.status.success());
    assert_eq!(std::fs::read(home.join("tally.db")).unwrap(), b"not really a database");
    assert_eq!(std::fs::read_dir(&home).unwrap().count(), 1);
}

fn win_with_extras(extras: &str) -> (Fake, PathBuf) {
    let mut w = fake(Os::Windows, r"C:\Users\Jo");
    w.path.insert("code".into(), r"C:\Tools\R&D\code.cmd".into());
    let extras = PathBuf::from(extras);
    w.files.get_mut().insert(extras.join("tally-vscode.vsix"));
    (w, extras)
}

#[test]
fn windows_command_line_quotes_every_path_even_without_spaces() {
    for dir in [r"C:\R&D\extras", r"C:\a^b\extras"] {
        let (w, extras) = win_with_extras(dir);
        go(&w, &opts(&extras));
        let log = w.log.borrow();
        let line = log.iter().map(|c| c[3].clone()).find(|l| l.contains("--install-extension")).unwrap();
        println!("{line}");
        let vsix = extras.join("tally-vscode.vsix").display().to_string();
        assert_eq!(line, format!("\"C:\\Tools\\R&D\\code.cmd\" --install-extension \"{vsix}\" --force"));
    }
}

#[test]
fn windows_arguments_that_cmd_would_expand_are_refused_and_run_nothing() {
    let (w, extras) = win_with_extras(r"C:\%PATH%\extras");
    let r = go(&w, &opts(&extras));
    assert!(outcome(&r, "VS Code").is_err());
    assert!(w.log.borrow().is_empty(), "{:?}", w.log.borrow());
}

#[test]
fn remove_never_uninstalls_something_that_is_not_listed() {
    let (mut e, extras) = linux_with_extras();
    e.path.insert("claude".into(), "claude".into());
    e.reply = Box::new(|c| if c.ends_with("list --json") { "[]".into() } else { String::new() });
    go(&e, &SetupOpts { remove: true, ..opts(&extras) });
    let cmds: Vec<String> = e.log.borrow().iter().map(|c| c.join(" ")).collect();
    assert!(cmds.iter().all(|c| !c.contains("uninstall") && !c.contains("marketplace remove")), "{cmds:?}");
}

#[test]
fn declining_the_prompt_changes_nothing() {
    let (mut e, extras) = linux_with_extras();
    e.path.insert("claude".into(), "claude".into());
    let o = SetupOpts { yes: false, ..opts(&extras) };
    let r = run(&e, &o, &mut std::io::Cursor::new(b"n\n".to_vec()), &mut std::io::sink()).unwrap();
    assert!(r.declined && r.rows.is_empty());
    assert!(e.log.borrow().is_empty());
}

#[test]
fn declining_with_include_wakatime_creates_no_token_file() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    let answers = t.path().join("answers");
    std::fs::write(&answers, "n\n").unwrap();
    let o = tally(&home, &["setup", "--include-wakatime"]).env_remove("TALLY_NO_TTY").env("TALLY_TTY_FILE", &answers).output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(String::from_utf8_lossy(&o.stdout).contains("nothing changed"));
    assert!(!home.exists(), "home was created");
}

#[test]
fn a_failed_cfg_write_leaves_the_original_bytes() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join(".wakatime.cfg");
    let old = "[settings]\napi_url = https://wakatime.com/api\n";
    std::fs::write(&p, old).unwrap();
    // The temp file's name is taken by a directory, so writing it fails.
    std::fs::create_dir(t.path().join(".wakatime.cfg.tally-tmp")).unwrap();
    assert!(matches!(write_cfg(&p, "http://127.0.0.1:1/api", "k", true), CfgResult::Conflict { .. }));
    assert_eq!(std::fs::read_to_string(&p).unwrap(), old);
}

#[test]
fn an_existing_cfg_without_our_keys_is_left_alone_unless_forced_and_then_backed_up() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join(".wakatime.cfg");
    let (url, key) = ("http://127.0.0.1:25930/api", "k");
    let old = "[settings]
debug = true
";
    std::fs::write(&p, old).unwrap();
    match write_cfg(&p, url, key, false) {
        CfgResult::Conflict { existing_url } => assert!(existing_url.contains("--force"), "{existing_url}"),
        r => panic!("{r:?}"),
    }
    assert_eq!(std::fs::read_to_string(&p).unwrap(), old);
    assert_eq!(write_cfg(&p, url, key, true), CfgResult::BackedUpAndWritten);
    assert_eq!(std::fs::read_to_string(t.path().join(".wakatime.cfg.tally-bak")).unwrap(), old);
    assert!(std::fs::read_to_string(&p).unwrap().contains("debug = true"));
}

#[test]
fn a_symlinked_cfg_is_never_replaced() {
    let t = tempfile::tempdir().unwrap();
    let real = t.path().join("real.cfg");
    std::fs::write(&real, "[settings]
").unwrap();
    let link = t.path().join(".wakatime.cfg");
    #[cfg(unix)]
    let ok = std::os::unix::fs::symlink(&real, &link).is_ok();
    #[cfg(windows)]
    let ok = std::os::windows::fs::symlink_file(&real, &link).is_ok(); // needs developer mode or admin
    if !ok {
        eprintln!("skipped: cannot create a symlink here");
        return;
    }
    assert!(matches!(write_cfg(&link, "http://127.0.0.1:1/api", "k", true), CfgResult::Conflict { .. }));
    assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
    assert_eq!(std::fs::read_to_string(&real).unwrap(), "[settings]
");
}

#[test]
fn an_unknown_only_name_is_an_error_listing_the_valid_names() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    let o = tally(&home, &["setup", "--yes", "--only", "vscod"]).output().unwrap();
    assert!(!o.status.success());
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("vscod") && err.contains("vscode") && err.contains("claude-code") && err.contains("jetbrains"), "{err}");
    assert!(!home.exists());
}

#[test]
fn is_loopback_url_accepts_and_rejects_like_the_extension() {
    use tally_daemon::setup::is_loopback_url as f;
    for u in ["http://127.0.0.1:25930/api", "http://localhost:1/api", "http://[::1]:1/api"] {
        assert!(f(u), "{u}");
    }
    for u in ["https://127.0.0.1:1/api", "http://example.com", "http://127.0.0.1.evil.com", "http://localhost.evil.com", "http://user@evil.com", "http://127.0.0.1@evil.com", "not a url", ""] {
        assert!(!f(u), "{u}");
    }
}
