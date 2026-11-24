//! `tally ensure`: what an editor or agent extension calls on every start. Records consent once,
//! registers the repo (local only) and starts a detached daemon unless one already holds `run.lock`.
//! Never fails the host: every error goes to `<home>/ensure.log`.
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub const EXPLAINER: &str = include_str!("explainer.txt");
const LOG_MAX: u64 = 64 * 1024;
const LOG_KEEP: u64 = 32 * 1024;

pub struct EnsureOpts {
    pub client: Option<String>,
    pub register_cwd: Option<PathBuf>,
}

pub struct EnsureOutcome {
    pub first_run: bool,
    pub started: bool,
    /// The explainer; set only when `first_run`.
    pub notice: Option<&'static str>,
}

/// True when a daemon holds `run.lock`. Otherwise the lock is taken and released again.
pub fn daemon_running(home: &Path) -> bool {
    match OpenOptions::new().create(true).write(true).truncate(false).open(home.join("run.lock")) {
        Ok(f) => f.try_lock().is_err(),
        Err(_) => false,
    }
}

fn log(home: &Path, msg: &str) {
    let _ = (|| -> std::io::Result<()> {
        let p = home.join("ensure.log");
        let mut f = OpenOptions::new().create(true).append(true).read(true).open(&p)?;
        let ts = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        writeln!(f, "{ts} {}", msg.replace('\n', " "))?;
        let len = f.metadata()?.len();
        if len > LOG_MAX {
            f.seek(SeekFrom::Start(len - LOG_KEEP))?;
            let mut tail = Vec::new();
            f.read_to_end(&mut tail)?;
            // Drop the partial first line.
            let from = tail.iter().position(|&b| b == b'\n').map_or(0, |i| i + 1);
            std::fs::write(&p, &tail[from..])?;
        }
        Ok(())
    })();
}

pub fn ensure(home: &Path, o: &EnsureOpts, spawn: &dyn Fn(&Path) -> std::io::Result<()>) -> EnsureOutcome {
    let mut out = EnsureOutcome { first_run: false, started: false, notice: None };
    if let Err(e) = std::fs::create_dir_all(home) {
        log(home, &format!("creating {}: {e}", home.display()));
        return out;
    }
    if home.join("paused").exists() {
        return out;
    }
    let consent = home.join("consent.json");
    if !consent.exists() {
        let body = serde_json::json!({
            "client": o.client.as_deref().unwrap_or("unknown"),
            "version": env!("CARGO_PKG_VERSION"),
            "at_ms": crate::source::now_ms(),
        });
        match std::fs::write(&consent, body.to_string()) {
            Ok(()) => {
                out.first_run = true;
                out.notice = Some(EXPLAINER);
            }
            Err(e) => log(home, &format!("writing consent.json: {e}")),
        }
    }
    if let Some(dir) = &o.register_cwd {
        if let Err(e) = register(home, dir) {
            log(home, &format!("registering {}: {e}", dir.display()));
        }
    }
    if !daemon_running(home) {
        match spawn(home) {
            Ok(()) => out.started = true,
            Err(e) => log(home, &format!("starting the daemon: {e}")),
        }
    }
    out
}

fn register(home: &Path, dir: &Path) -> Result<(), String> {
    let Some((root, _)) = crate::project::project_for_dir(dir) else { return Ok(()) };
    let mut store = crate::store::Store::open(&home.join("tally.db")).map_err(|e| e.to_string())?;
    crate::project::register_git_project(&mut store, &root, None).map(|_| ()).map_err(|e| e.to_string())
}

/// Starts `<current exe> run --home <home>` detached, output appended to `<home>/tally.log`; does not wait.
pub fn spawn_daemon(home: &Path) -> std::io::Result<()> {
    use std::process::{Command, Stdio};
    let logf = OpenOptions::new().create(true).append(true).open(home.join("tally.log"))?;
    let mut c = Command::new(std::env::current_exe()?);
    c.arg("run").arg("--home").arg(home).stdin(Stdio::null()).stdout(logf.try_clone()?).stderr(logf);
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut c, 0x0800_0000 | 0x0000_0008); // CREATE_NO_WINDOW | DETACHED_PROCESS
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut c, 0);
    c.spawn().map(|_| ())
}
