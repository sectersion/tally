use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tally_daemon::service::*;
use tally_daemon::service_exec::*;

#[derive(Default)]
struct Fake {
    log: Vec<String>,
    fail_programs: Vec<&'static str>,
}

fn out(code: i32, err: &str) -> std::process::Output {
    #[cfg(windows)]
    let status = std::os::windows::process::ExitStatusExt::from_raw(code as u32);
    #[cfg(unix)]
    let status = std::os::unix::process::ExitStatusExt::from_raw(code << 8);
    std::process::Output { status, stdout: vec![], stderr: err.as_bytes().to_vec() }
}

impl Runner for Fake {
    fn run(&mut self, c: &Cmd) -> io::Result<std::process::Output> {
        self.log.push(format!("run {} {}", c.program, c.args.join(" ")));
        Ok(if self.fail_programs.contains(&c.program.as_str()) { out(1, "not found") } else { out(0, "") })
    }
    fn write(&mut self, p: &Path, _: &str) -> io::Result<()> {
        self.log.push(format!("write {}", p.display()));
        Ok(())
    }
    fn remove(&mut self, p: &Path) -> io::Result<()> {
        self.log.push(format!("remove {}", p.display()));
        Err(io::ErrorKind::NotFound.into())
    }
    fn remove_dir_all(&mut self, p: &Path) -> io::Result<()> {
        self.log.push(format!("rmdir {}", p.display()));
        std::fs::remove_dir_all(p)
    }
}

fn spec(home: &Path) -> ServiceSpec {
    ServiceSpec { exe: "/usr/local/bin/tally".into(), home: home.into(), label: LABEL, unit_dir: PathBuf::from("/u"), uid: 1, user: "D\\o".into() }
}

fn purge(yes: bool, daemon_up: bool) -> PurgeOpts<'static> {
    PurgeOpts { yes, confirm: &|| false, daemon_running: if daemon_up { &|| true } else { &|| false }, wait: Duration::from_millis(50) }
}

fn home_with_db() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("tally.db"), b"x").unwrap();
    d
}

#[test]
fn uninstall_keeps_the_data_directory() {
    let d = home_with_db();
    let log = uninstall("linux", &spec(d.path()), &mut Fake::default(), None).unwrap();
    assert!(d.path().join("tally.db").exists());
    assert!(log.last().unwrap().starts_with("data kept in"));
}

#[test]
fn purge_without_yes_noninteractive_refuses_and_deletes_nothing() {
    let d = home_with_db();
    let mut f = Fake::default();
    let e = uninstall("linux", &spec(d.path()), &mut f, Some(&purge(false, false))).unwrap_err();
    assert!(matches!(e, ExecError::Refused(_)));
    assert!(d.path().join("tally.db").exists());
    assert!(f.log.is_empty(), "{:?}", f.log);
}

#[test]
fn purge_with_yes_removes_home_after_the_service() {
    let d = home_with_db();
    let mut f = Fake::default();
    uninstall("linux", &spec(d.path()), &mut f, Some(&purge(true, false))).unwrap();
    assert!(!d.path().exists());
    let rm = f.log.iter().position(|l| l.starts_with("rmdir")).unwrap();
    let svc = f.log.iter().position(|l| l.contains("disable --now")).unwrap();
    assert!(svc < rm, "{:?}", f.log);
}

#[test]
fn uninstall_when_not_installed_succeeds() {
    let d = home_with_db();
    let mut f = Fake { fail_programs: vec!["systemctl", "launchctl", "schtasks"], ..Default::default() };
    for os in ["linux", "macos", "windows"] {
        uninstall(os, &spec(d.path()), &mut f, None).unwrap();
    }
}

#[test]
fn install_twice_is_ok_and_leaves_one_definition() {
    let d = home_with_db();
    let mut f = Fake::default();
    for _ in 0..2 {
        execute(&plan("linux", Action::Install, &spec(d.path())), &mut f).unwrap();
    }
    let writes: std::collections::HashSet<_> = f.log.iter().filter(|l| l.starts_with("write")).collect();
    assert_eq!(writes.len(), 1);
}

#[test]
fn a_failing_required_command_stops_with_its_stderr() {
    let d = home_with_db();
    let mut f = Fake { fail_programs: vec!["systemctl"], ..Default::default() };
    let e = execute(&plan("linux", Action::Install, &spec(d.path())), &mut f).unwrap_err();
    assert!(e.to_string().contains("systemctl --user daemon-reload") && e.to_string().contains("not found"), "{e}");
}

#[test]
fn status_exit_codes_with_quiet() {
    assert_eq!(ServiceState::of(true, true).exit_code(), 0);
    assert_eq!(ServiceState::of(true, false).exit_code(), 3);
    assert_eq!(ServiceState::of(false, false).exit_code(), 4);
    assert_eq!(ServiceState::of(false, true).text(), "running");
}

#[test]
fn build_directory_executables_are_flagged() {
    assert!(is_build_path(Path::new(r"C:\x\tally\target\debug\tally.exe")));
    assert!(is_build_path(Path::new("/x/target/release/tally")));
    assert!(!is_build_path(Path::new("/usr/local/bin/tally")));
}

#[test]
fn purge_refuses_while_the_daemon_still_holds_the_home() {
    let d = home_with_db();
    let mut f = Fake::default();
    let e = uninstall("linux", &spec(d.path()), &mut f, Some(&purge(true, true))).unwrap_err();
    assert!(matches!(e, ExecError::Refused(_)), "{e}");
    assert!(d.path().join("tally.db").exists());
    assert!(!f.log.iter().any(|l| l.starts_with("rmdir") || l.starts_with("remove")), "{:?}", f.log);
}
