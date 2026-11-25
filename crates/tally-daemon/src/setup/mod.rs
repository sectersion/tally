//! `tally setup`: detect editors and clients, install their extensions. `Env` is the only seam to the machine.
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::Output;

pub mod claude_code;
pub mod vscode;
pub mod wakatime;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    Windows,
    MacOs,
    Linux,
}

pub trait Env {
    fn os(&self) -> Os;
    fn home(&self) -> PathBuf;
    fn which(&self, name: &str) -> Option<PathBuf>;
    fn exists(&self, p: &Path) -> bool;
    fn var(&self, k: &str) -> Option<String>;
    fn run(&self, program: &Path, args: &[String]) -> std::io::Result<Output>;
}

pub struct RealEnv;

impl Env for RealEnv {
    fn os(&self) -> Os {
        if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::MacOs
        } else {
            Os::Linux
        }
    }
    fn home(&self) -> PathBuf {
        dirs::home_dir().unwrap_or_default()
    }
    fn which(&self, name: &str) -> Option<PathBuf> {
        let exts: &[&str] = if cfg!(windows) { &[".cmd", ".exe", ".bat"] } else { &[""] };
        std::env::split_paths(&std::env::var_os("PATH")?)
            .flat_map(|d| exts.iter().map(move |x| d.join(format!("{name}{x}"))))
            .find(|p| p.is_file())
    }
    fn exists(&self, p: &Path) -> bool {
        p.exists()
    }
    fn var(&self, k: &str) -> Option<String> {
        std::env::var(k).ok()
    }
    fn run(&self, program: &Path, args: &[String]) -> std::io::Result<Output> {
        let mut c = std::process::Command::new(program);
        #[cfg(windows)]
        if program == Path::new("cmd") && args.len() == 3 && args[0] == "/S" {
            // `cmd /S /C "<one command line>"`: std would re-quote the line and cmd would misread it, so pass it raw.
            std::os::windows::process::CommandExt::raw_arg(c.arg("/S").arg("/C"), format!("\"{}\"", args[2]));
            return c.stdin(std::process::Stdio::null()).output();
        }
        c.args(args).stdin(std::process::Stdio::null()).output()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Detection {
    Found { cli: PathBuf },
    ConfigOnly { hint: String },
    NotFound,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Installed,
    AlreadyInstalled,
    Skipped(String),
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct SetupError(pub String);

pub struct Ctx {
    pub extras_dir: PathBuf,
    pub dry_run: bool,
    pub force: bool,
}

pub trait Integration {
    fn name(&self) -> String;
    fn detect(&self, e: &dyn Env) -> Detection;
    fn install(&self, e: &dyn Env, ctx: &Ctx) -> Result<Outcome, SetupError>;
    fn remove(&self, e: &dyn Env) -> Result<Outcome, SetupError>;
}

pub fn registry(include_wakatime: bool) -> Vec<Box<dyn Integration>> {
    let mut v = vscode::family();
    v.push(Box::new(claude_code::ClaudeCode));
    if include_wakatime {
        v.push(Box::new(wakatime::JetBrains));
    }
    v
}

#[derive(Default)]
pub struct SetupOpts {
    pub yes: bool,
    pub dry_run: bool,
    pub only: Vec<String>,
    pub include_wakatime: bool,
    pub force: bool,
    pub remove: bool,
    pub extras_dir: PathBuf,
    /// Yields `(api_url, api_key)` for `~/.wakatime.cfg`; called only after consent and a JetBrains install (it may create the token file).
    pub wakatime_cfg: Option<CfgSource>,
}

/// Produces `(api_url, api_key)`.
pub type CfgSource = Box<dyn Fn() -> Option<(String, String)>>;

pub struct Row {
    pub name: String,
    pub detection: Detection,
    /// `None`: not selected.
    pub outcome: Option<Result<Outcome, String>>,
}

pub struct Report {
    pub rows: Vec<Row>,
    /// The user declined `Continue?`.
    pub declined: bool,
}

/// cmd expands %VAR% even inside quotes and cannot carry a quote or control character: refuse, never escape.
pub fn check_cmd_args<'a>(os: Os, args: impl IntoIterator<Item = &'a str>) -> std::io::Result<()> {
    match args.into_iter().find(|a| os == Os::Windows && a.chars().any(|c| c == '%' || c == '"' || c.is_control())) {
        Some(bad) => Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("refusing to pass {bad:?} through cmd.exe"))),
        None => Ok(()),
    }
}

/// Runs `cli args...`; on Windows through `cmd /S /C` because the CLIs of the VS Code family are `.cmd` files.
/// The recorded argv is `cmd /S /C <one command line>`; `RealEnv::run` wraps that line in the outer quotes `/S` strips.
pub fn run_cli(e: &dyn Env, cli: &Path, args: &[&str]) -> std::io::Result<Output> {
    if e.os() != Os::Windows {
        return e.run(cli, &args.iter().map(|s| s.to_string()).collect::<Vec<_>>());
    }
    let cli = cli.display().to_string();
    check_cmd_args(e.os(), std::iter::once(cli.as_str()).chain(args.iter().copied()))?;
    // Quote the CLI and everything that is not a plain word, so `&`, `^`, `(` and spaces in paths stay data.
    let q = |s: &str| if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "._@=-".contains(c)) { s.to_string() } else { format!("\"{s}\"") };
    let line = std::iter::once(format!("\"{cli}\"")).chain(args.iter().map(|a| q(a))).collect::<Vec<_>>().join(" ");
    e.run(Path::new("cmd"), &["/S".into(), "/C".into(), line])
}

/// PATH, then the known install paths, then a config folder alone (`ConfigOnly`).
pub fn detect_cli(e: &dyn Env, cli: &str, config_dir: &str, mac_app: &str, win_folder: &str) -> Detection {
    if let Some(p) = e.which(cli) {
        return Detection::Found { cli: p };
    }
    let home = e.home();
    let mut cands: Vec<PathBuf> = Vec::new();
    match e.os() {
        Os::MacOs => {
            let rel = format!("{mac_app}.app/Contents/Resources/app/bin/{cli}");
            cands.push(Path::new("/Applications").join(&rel));
            cands.push(home.join("Applications").join(&rel));
        }
        Os::Linux => {
            for d in ["/usr/bin", "/usr/local/bin", "/snap/bin"] {
                cands.push(Path::new(d).join(cli));
            }
            cands.push(home.join(".local/bin").join(cli));
        }
        Os::Windows => {
            for (var, sub) in [("LOCALAPPDATA", "Programs"), ("ProgramFiles", "")] {
                if let Some(b) = e.var(var) {
                    let b = Path::new(&b).join(sub);
                    cands.push(b.join(win_folder).join("bin").join(format!("{cli}.cmd")));
                }
            }
        }
    }
    if let Some(p) = cands.into_iter().find(|p| e.exists(p)) {
        return Detection::Found { cli: p };
    }
    if e.exists(&home.join(config_dir)) {
        return Detection::ConfigOnly { hint: format!("run 'Shell Command: Install {cli} command in PATH' in the editor") };
    }
    Detection::NotFound
}

fn key(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase()
}

fn ask(input: &mut dyn BufRead, out: &mut dyn Write, q: &str) -> String {
    let _ = write!(out, "{q}");
    let _ = out.flush();
    let mut s = String::new();
    let _ = input.read_line(&mut s);
    s.trim().to_string()
}

pub fn run(e: &dyn Env, o: &SetupOpts, input: &mut dyn BufRead, out: &mut dyn Write) -> Result<Report, SetupError> {
    let names: Vec<String> = registry(true).iter().map(|i| i.name()).collect();
    if let Some(bad) = o.only.iter().find(|n| !names.iter().any(|k| key(k) == key(n))) {
        return Err(SetupError(format!("unknown --only name {bad:?}; valid names: {}", names.iter().map(|n| n.to_lowercase().replace("vs code", "vscode").replace(' ', "-")).collect::<Vec<_>>().join(", "))));
    }
    let _ = writeln!(out, "{}", crate::ensure::EXPLAINER);
    let mut rep = Report { rows: vec![], declined: false };
    if !o.yes && !o.dry_run && !ask(input, out, "Continue? [y/N] ").to_lowercase().starts_with('y') {
        rep.declined = true;
        return Ok(rep);
    }
    let only: Vec<String> = o.only.iter().map(|s| key(s)).collect();
    let all = registry(o.include_wakatime);
    let mut found = Vec::new();
    for i in all.iter().filter(|i| only.is_empty() || only.contains(&key(&i.name()))) {
        let d = i.detect(e);
        let _ = match &d {
            Detection::Found { cli } => writeln!(out, "{}: found ({})", i.name(), cli.display()),
            Detection::ConfigOnly { hint } => writeln!(out, "{}: config only: {hint}", i.name()),
            Detection::NotFound => writeln!(out, "{}: not found", i.name()),
        };
        if matches!(d, Detection::Found { .. }) {
            found.push(rep.rows.len());
        }
        rep.rows.push(Row { name: i.name(), detection: d, outcome: None });
    }
    if found.len() > 1 && only.is_empty() && !o.yes && !o.dry_run {
        let a = ask(input, out, "Install into which? names, comma separated (Enter = all): ");
        if !a.is_empty() {
            let want: Vec<String> = a.split(',').map(key).collect();
            found.retain(|&n| want.contains(&key(&rep.rows[n].name)));
        }
    }
    let ctx = Ctx { extras_dir: o.extras_dir.clone(), dry_run: o.dry_run, force: o.force };
    for n in found {
        let i = all.iter().find(|i| i.name() == rep.rows[n].name).expect("registered");
        let r = if o.dry_run {
            Ok(Outcome::Skipped(format!("would {}", if o.remove { "remove" } else { "install" })))
        } else if o.remove {
            i.remove(e)
        } else {
            i.install(e, &ctx)
        };
        if let (Ok(Outcome::Installed | Outcome::AlreadyInstalled), false, Some(f)) = (&r, o.remove || o.dry_run, &o.wakatime_cfg) {
            if let (true, Some((u, k))) = (i.name() == wakatime::JETBRAINS, f()) {
                let p = e.home().join(".wakatime.cfg");
                let _ = writeln!(out, "  {}: {:?}", p.display(), wakatime::write_cfg(&p, &u, &k, o.force));
            }
        }
        let r = r.map_err(|x| x.0);
        let _ = match &r {
            Ok(x) => writeln!(out, "  {}: {x:?}", rep.rows[n].name),
            Err(m) => writeln!(out, "  {}: error: {m}", rep.rows[n].name),
        };
        rep.rows[n].outcome = Some(r);
    }
    Ok(rep)
}

/// Loopback only, plain http, no credentials: the same rule as `isLoopbackUrl` in the VS Code extension.
pub fn is_loopback_url(s: &str) -> bool {
    let Some(rest) = s.strip_prefix("http://") else { return false };
    let auth = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = if auth.starts_with('[') { auth.split(']').next().map(|h| format!("{h}]")).unwrap_or_default() } else { auth.split(':').next().unwrap_or("").to_string() };
    !auth.contains('@') && ["127.0.0.1", "localhost", "[::1]"].contains(&host.as_str())
}

/// After the installs: start the daemon (`ensure` records consent), send one test heartbeat to the local endpoint.
pub fn finish(home: &Path, out: &mut dyn Write) -> anyhow::Result<()> {
    use anyhow::Context;
    let o = crate::ensure::EnsureOpts { client: Some("setup".into()), register_cwd: None };
    crate::ensure::ensure(home, &o, &crate::ensure::spawn_daemon);
    let cfg = crate::config::load(home)?;
    let tok = cfg.heartbeat.token_file.clone().unwrap_or_else(|| home.join("heartbeat.token"));
    let key = crate::heartbeat::server::load_or_create_token(&tok).context("reading the heartbeat token")?;
    let url = format!("http://127.0.0.1:{}/api/users/current/heartbeats", cfg.heartbeat.port);
    anyhow::ensure!(is_loopback_url(&url), "refusing to send to a non-loopback url");
    let body = serde_json::json!({"entity": "tally-setup-test", "type": "file", "time": crate::source::now_ms() as f64 / 1000.0, "plugin": "tally-setup"});
    let auth = format!("Basic {}", base64::Engine::encode(&base64::engine::general_purpose::STANDARD, key));
    let agent = ureq::AgentBuilder::new().timeout(std::time::Duration::from_secs(3)).build();
    let t0 = std::time::Instant::now();
    let status = loop {
        match agent.post(&url).set("Authorization", &auth).set("Content-Type", "application/json").send_string(&body.to_string()) {
            Ok(r) => break Some(r.status()),
            Err(ureq::Error::Status(c, _)) => break Some(c),
            Err(_) if t0.elapsed().as_secs() < 10 => std::thread::sleep(std::time::Duration::from_millis(250)),
            Err(_) => break None,
        }
    };
    let _ = match status {
        Some(c) => writeln!(out, "test heartbeat: {c}"),
        None => writeln!(out, "test heartbeat: no answer from the local heartbeat server"),
    };
    let state = if crate::ensure::daemon_running(home) { "running" } else if home.join("paused").exists() { "paused (run `tally start`)" } else { "not running" };
    let _ = writeln!(out, "daemon: {state}");
    Ok(())
}
