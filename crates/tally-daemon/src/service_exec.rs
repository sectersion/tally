//! Executes service plans through a `Runner` (the only seam to the machine, so tests use a recording fake).
use crate::service::{plan, Action, Cmd, ServiceSpec, Step};
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum ExecError {
    #[error("`{cmd}` failed: {stderr}")]
    Command { cmd: String, stderr: String },
    #[error("{0}")]
    Io(String),
    #[error("{0}")]
    Refused(String),
}

pub trait Runner {
    fn run(&mut self, c: &Cmd) -> std::io::Result<std::process::Output>;
    fn write(&mut self, p: &Path, contents: &str) -> std::io::Result<()>;
    fn remove(&mut self, p: &Path) -> std::io::Result<()>;
    fn remove_dir_all(&mut self, p: &Path) -> std::io::Result<()>;
}

pub struct RealRunner;

impl Runner for RealRunner {
    fn run(&mut self, c: &Cmd) -> std::io::Result<std::process::Output> {
        std::process::Command::new(&c.program).args(&c.args).stdin(std::process::Stdio::null()).output()
    }
    fn write(&mut self, p: &Path, contents: &str) -> std::io::Result<()> {
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d)?;
        }
        if p.extension().is_some_and(|e| e == "xml") {
            // schtasks /XML needs UTF-16 with a BOM
            let mut b = vec![0xFF, 0xFE];
            b.extend(contents.encode_utf16().flat_map(u16::to_le_bytes));
            std::fs::write(p, b)
        } else {
            std::fs::write(p, contents)
        }
    }
    fn remove(&mut self, p: &Path) -> std::io::Result<()> {
        std::fs::remove_file(p)
    }
    fn remove_dir_all(&mut self, p: &Path) -> std::io::Result<()> {
        std::fs::remove_dir_all(p)
    }
}

fn show(c: &Cmd) -> String {
    format!("{} {}", c.program, c.args.join(" "))
}

/// Runs the steps in order and returns what it did. `TryRun` failures and a missing file on remove are not errors.
pub fn execute(steps: &[Step], r: &mut dyn Runner) -> Result<Vec<String>, ExecError> {
    let mut log = vec![];
    for st in steps {
        match st {
            Step::WriteFile { path, contents } => {
                r.write(path, contents).map_err(|e| ExecError::Io(format!("writing {}: {e}", path.display())))?;
                log.push(format!("wrote {}", path.display()));
            }
            Step::Run(c) => {
                let out = r.run(c).map_err(|e| ExecError::Command { cmd: show(c), stderr: e.to_string() })?;
                if !out.status.success() {
                    return Err(ExecError::Command { cmd: show(c), stderr: String::from_utf8_lossy(&out.stderr).trim().to_string() });
                }
                log.push(show(c));
            }
            Step::TryRun(c) => {
                let _ = r.run(c);
                log.push(format!("{} (failure ignored)", show(c)));
            }
            Step::RemoveFile(path) => match r.remove(path) {
                Ok(()) => log.push(format!("removed {}", path.display())),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(ExecError::Io(format!("removing {}: {e}", path.display()))),
            },
        }
    }
    Ok(log)
}

pub struct PurgeOpts<'a> {
    pub yes: bool,
    /// Asked only when `yes` is false (an interactive "type delete" prompt; `|| false` off a terminal).
    pub confirm: &'a dyn Fn() -> bool,
    /// Whether the daemon still holds the data directory (its run lock).
    pub daemon_running: &'a dyn Fn() -> bool,
    /// How long to wait for the daemon to exit after the stop request.
    pub wait: std::time::Duration,
    /// True for the user's own home directory (the real one comes from `dirs::home_dir()`).
    pub is_user_home: &'a dyn Fn(&Path) -> bool,
}

/// `--purge` deletes `home` recursively and `--home`/`TALLY_HOME` can name anything. Only a directory that already holds
/// Tally's own files, is not the user's home and holds no git repository qualifies.
fn purge_target_problem(home: &Path, is_user_home: &dyn Fn(&Path) -> bool) -> Option<String> {
    if !home.is_absolute() {
        return Some(format!("{} is a relative path", home.display()));
    }
    if is_user_home(home) {
        return Some(format!("{} is your home directory", home.display()));
    }
    if !home.join("tally.db").exists() && !home.join("config.toml").exists() {
        return Some(format!("{} is not a Tally data directory (no tally.db or config.toml in it)", home.display()));
    }
    if home.join(".git").exists() {
        return Some(format!("{} holds a git repository", home.display()));
    }
    None
}

/// Removes the service registration; with `purge` also deletes `home`, last, after consent and only once the
/// daemon has exited (deleting around a live daemon half-deletes the store and breaks its lock).
pub fn uninstall(os: &str, s: &ServiceSpec, r: &mut dyn Runner, purge: Option<&PurgeOpts>) -> Result<Vec<String>, ExecError> {
    if let Some(p) = purge {
        if !p.yes && !(p.confirm)() {
            return Err(ExecError::Refused(format!("--purge deletes {}; pass --yes to confirm (nothing was changed)", s.home.display())));
        }
        if let Some(why) = purge_target_problem(&s.home, p.is_user_home) {
            return Err(ExecError::Refused(format!("refusing to purge: {why}; nothing was changed")));
        }
        // stop the daemon first so it does not hold files open; not running is fine
        let stop: Vec<Step> = plan(os, Action::Stop, s).into_iter().map(|st| if let Step::Run(c) = st { Step::TryRun(c) } else { st }).collect();
        let _ = execute(&stop, r);
        let t0 = std::time::Instant::now();
        while (p.daemon_running)() && t0.elapsed() < p.wait {
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        if (p.daemon_running)() {
            return Err(ExecError::Refused(format!("the daemon is still running; run `tally stop`, wait for it to exit, then retry (nothing was deleted from {})", s.home.display())));
        }
    }
    let mut log = execute(&plan(os, Action::Uninstall, s), r)?;
    if purge.is_some() {
        // the daemon releases its lock slightly before its last file handles (seen on Windows): retry briefly
        let mut res = r.remove_dir_all(&s.home);
        for _ in 0..10 {
            if res.is_ok() || !s.home.exists() {
                res = Ok(());
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(300));
            res = r.remove_dir_all(&s.home);
        }
        res.map_err(|e| ExecError::Io(format!("deleting {}: {e}", s.home.display())))?;
        log.push(format!("deleted {}", s.home.display()));
    } else {
        log.push(format!("data kept in {}", s.home.display()));
    }
    Ok(log)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceState {
    Running,
    InstalledStopped,
    NotInstalled,
}

impl ServiceState {
    pub fn of(installed: bool, running: bool) -> Self {
        match (installed, running) {
            (_, true) => Self::Running,
            (true, false) => Self::InstalledStopped,
            (false, false) => Self::NotInstalled,
        }
    }
    pub fn text(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::InstalledStopped => "installed, not running",
            Self::NotInstalled => "not installed",
        }
    }
    /// For `status --quiet`: 0 running, 3 stopped, 4 not installed.
    pub fn exit_code(self) -> i32 {
        match self {
            Self::Running => 0,
            Self::InstalledStopped => 3,
            Self::NotInstalled => 4,
        }
    }
}

/// A service pointing at a build directory breaks on the next `cargo clean`.
pub fn is_build_path(exe: &Path) -> bool {
    let s = exe.to_string_lossy().replace('\\', "/");
    s.contains("target/debug") || s.contains("target/release")
}
