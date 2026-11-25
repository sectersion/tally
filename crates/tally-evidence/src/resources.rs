//! Measures the daemon's OWN memory and CPU (the harness and the load writer are other processes and are excluded)
//! and writes a numbers file that states pass or fail against the pitch targets next to each measured figure.
use crate::load::{write_session_log, LoadPlan, RealClock};
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use sysinfo::{Pid, ProcessesToUpdate, System};

/// PITCH: under 50 MB of RAM and under 1% CPU during active use.
pub const RSS_TARGET_BYTES: u64 = 50_000_000;
pub const CPU_TARGET_X100: u32 = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample {
    pub t_s: u64,
    pub rss_bytes: u64,
    /// Percent of one core times 100 (so 100 = 1%). Can exceed 10000 on multi-core.
    pub cpu_pct_x100: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub n: usize,
    pub rss_p50: u64,
    pub rss_p95: u64,
    pub rss_max: u64,
    pub cpu_mean_x100: u32,
    pub cpu_p95_x100: u32,
    pub cpu_max_x100: u32,
}

/// Nearest-rank percentile of an ascending slice.
fn pct<T: Copy + Default>(sorted: &[T], p: usize) -> T {
    if sorted.is_empty() {
        return T::default();
    }
    sorted[(p * sorted.len()).div_ceil(100).clamp(1, sorted.len()) - 1]
}

pub fn summarize(s: &[Sample]) -> Summary {
    let mut rss: Vec<u64> = s.iter().map(|x| x.rss_bytes).collect();
    let mut cpu: Vec<u32> = s.iter().map(|x| x.cpu_pct_x100).collect();
    rss.sort_unstable();
    cpu.sort_unstable();
    Summary {
        n: s.len(),
        rss_p50: pct(&rss, 50),
        rss_p95: pct(&rss, 95),
        rss_max: rss.last().copied().unwrap_or(0),
        cpu_mean_x100: if cpu.is_empty() { 0 } else { (cpu.iter().map(|&c| u64::from(c)).sum::<u64>() / cpu.len() as u64) as u32 },
        cpu_p95_x100: pct(&cpu, 95),
        cpu_max_x100: cpu.last().copied().unwrap_or(0),
    }
}

#[derive(Clone, Debug)]
pub struct MachineInfo {
    /// From `--machine`: make, model, year, as the person measuring wrote it. Required.
    pub description: String,
    pub os: String,
    pub cpu_model: String,
    pub logical_cores: usize,
    pub total_ram_bytes: u64,
}

impl MachineInfo {
    pub fn detect(description: &str) -> Result<Self> {
        if description.trim().is_empty() {
            bail!("--machine is required: describe the machine (make, model, year, CPU, RAM, Windows build); numbers without it are not published");
        }
        let mut sys = System::new();
        sys.refresh_cpu_all();
        sys.refresh_memory();
        Ok(Self {
            description: description.trim().to_string(),
            os: format!("{} {}", System::name().unwrap_or_default(), System::os_version().unwrap_or_default()),
            cpu_model: sys.cpus().first().map(|c| c.brand().to_string()).unwrap_or_default(),
            logical_cores: sys.cpus().len(),
            total_ram_bytes: sys.total_memory(),
        })
    }
}

fn verdict(ok: bool) -> &'static str {
    if ok {
        "PASS"
    } else {
        "FAIL"
    }
}

fn row(name: &str, s: &Summary, target_note: &str) -> String {
    format!(
        "| {name} | {} | {:.1} MB | {:.1} MB | {:.1} MB | {:.2}% | {:.2}% | {:.2}% | {target_note} |\n",
        s.n,
        s.rss_p50 as f64 / 1e6,
        s.rss_p95 as f64 / 1e6,
        s.rss_max as f64 / 1e6,
        f64::from(s.cpu_mean_x100) / 100.0,
        f64::from(s.cpu_p95_x100) / 100.0,
        f64::from(s.cpu_max_x100) / 100.0
    )
}

/// `idle` and `active` phases are reported separately; the targets are checked against `active`.
pub fn render_numbers(idle: &Summary, active: &Summary, m: &MachineInfo, load: &LoadPlan, version: &str, plugged: Option<bool>) -> String {
    let mut s = format!(
        "# Resource numbers\n\nMachine: **{}**\n\n- OS: {}\n- CPU: {} ({} logical cores)\n- RAM: {:.1} GB\n- Tally: {version}\n- Power: {}\n- Load: {} h simulated, {} prompts/hour, {} tool events per prompt{}\n\nMeasured on the daemon process only (`tally run`); the harness and the load writer are excluded. CPU is percent of ONE core.\n\n| phase | samples | RSS p50 | RSS p95 | RSS max | CPU mean | CPU p95 | CPU max | |\n|---|---|---|---|---|---|---|---|---|\n",
        m.description,
        m.os,
        m.cpu_model,
        m.logical_cores,
        m.total_ram_bytes as f64 / 1e9,
        match plugged {
            Some(true) => "plugged in",
            Some(false) => "on battery",
            None => "not recorded",
        },
        load.hours,
        load.prompts_per_hour,
        load.tool_events_per_prompt,
        if load.speedup > 1 { format!(", accelerated {}x (NOT a real-time run)", load.speedup) } else { String::new() },
    );
    s.push_str(&row("idle", idle, ""));
    s.push_str(&row("active", active, ""));
    s.push_str(&format!(
        "\n## Targets (PITCH: under 50 MB of RAM, under 1% CPU during active use)\n\n- RSS p95 (active): {:.1} MB: **{}** (target < 50 MB)\n- CPU p95 (active): {:.2}%: **{}** (target < 1%)\n",
        active.rss_p95 as f64 / 1e6,
        verdict(active.rss_p95 < RSS_TARGET_BYTES),
        f64::from(active.cpu_p95_x100) / 100.0,
        verdict(active.cpu_p95_x100 < CPU_TARGET_X100),
    ));
    s
}

/// Samples one process every `every` for `duration`.
pub fn measure(child_pid: u32, duration: Duration, every: Duration) -> Vec<Sample> {
    let pid = Pid::from_u32(child_pid);
    let mut sys = System::new();
    let t0 = Instant::now();
    let mut out = vec![];
    sys.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
    while t0.elapsed() < duration {
        std::thread::sleep(every);
        sys.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
        match sys.process(pid) {
            Some(p) => out.push(Sample { t_s: t0.elapsed().as_secs(), rss_bytes: p.memory(), cpu_pct_x100: (p.cpu_usage() * 100.0).round() as u32 }),
            None => break, // the daemon died: stop sampling, the caller reports it
        }
    }
    out
}

pub struct Options {
    pub tally: PathBuf,
    pub machine: String,
    pub duration: Duration,
    /// 1 = real time, N = accelerated.
    pub speedup: u64,
    pub out: PathBuf,
    /// For CI runners that have no idle source.
    pub no_presence: bool,
    pub plugged: Option<bool>,
}

/// The `events: N` line of `tally status`.
pub fn events_in_status(text: &str) -> Option<u64> {
    text.lines().find_map(|l| l.strip_prefix("events: ")?.trim().parse().ok())
}

pub fn check_ingested(events: Option<u64>) -> Result<()> {
    match events {
        Some(0) => bail!("the daemon ingested no events from the load (the adapter ignored it or the writer wrote nothing); no numbers were written"),
        Some(_) => Ok(()),
        None => bail!("could not read the event count from `tally status`; no numbers were written"),
    }
}

/// Idle phase: the first tenth of the run, at most 10 minutes.
fn idle_phase(total: Duration) -> Duration {
    (total / 6).min(Duration::from_secs(600))
}

pub fn run(o: &Options) -> Result<Summary> {
    let machine = MachineInfo::detect(&o.machine)?; // before anything is spawned or written
    let tmp = std::env::temp_dir().join(format!("tally-resources-{}", std::process::id()));
    let (home, logs, repo) = (tmp.join("home"), tmp.join("logs"), tmp.join("repo"));
    for d in [&home, &logs, &repo] {
        std::fs::create_dir_all(d)?;
    }
    let _ = std::process::Command::new("git").arg("init").arg("-q").arg(&repo).status();
    let mut cfg = format!("[claude_code]\nprojects_dir = {}\n", toml_str(&logs));
    if o.no_presence {
        cfg.push_str("[presence]\nenabled = false\nrequire = false\n");
    }
    std::fs::write(home.join("config.toml"), cfg)?;

    let mut child = std::process::Command::new(&o.tally)
        .arg("--home")
        .arg(&home)
        .arg("run")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .with_context(|| format!("starting {}", o.tally.display()))?;
    let pid = child.id();
    let version = std::process::Command::new(&o.tally).arg("--version").output().map(|x| String::from_utf8_lossy(&x.stdout).trim().to_string()).unwrap_or_default();
    let result = (|| -> Result<(Summary, Summary, LoadPlan)> {
        let idle_for = idle_phase(o.duration);
        let idle = summarize(&measure(pid, idle_for, Duration::from_secs(1)));
        let mut plan = LoadPlan::default_for(repo.clone());
        plan.speedup = o.speedup;
        // enough simulated work to outlast the run: the writer is stopped with the process
        plan.hours = ((o.duration.as_secs() * o.speedup) / 3600 + 1) as u32;
        let (logs2, plan2) = (logs.clone(), plan.clone());
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        std::thread::spawn(move || {
            if let Err(e) = write_session_log(&logs2, &plan2, &mut RealClock) {
                let _ = tx.send(e.to_string());
            }
        });
        let active_samples = measure(pid, o.duration.saturating_sub(idle_for), Duration::from_secs(1));
        if child.try_wait()?.is_some() {
            bail!("the daemon exited during the measurement; no numbers were written");
        }
        if let Ok(e) = rx.try_recv() {
            bail!("the load writer failed ({e}); the active phase would be an idle daemon's numbers, so none were written");
        }
        // The daemon must really have ingested the load, or "active" would be an idle daemon reported as PASS.
        let status = std::process::Command::new(&o.tally).arg("--home").arg(&home).arg("status").output().context("running `tally status`")?;
        check_ingested(events_in_status(&String::from_utf8_lossy(&status.stdout)))?;
        if active_samples.is_empty() {
            bail!("no samples collected");
        }
        Ok((idle, summarize(&active_samples), plan))
    })();
    let _ = child.kill();
    let _ = child.wait();
    let (idle, active, plan) = result.inspect_err(|_| {
        let _ = std::fs::remove_dir_all(&tmp);
    })?;
    let md = render_numbers(&idle, &active, &machine, &plan, &version, o.plugged);
    std::fs::write(&o.out, md).with_context(|| format!("writing {}", o.out.display()))?;
    let _ = std::fs::remove_dir_all(&tmp);
    Ok(active)
}

fn toml_str(p: &Path) -> String {
    format!("'{}'", p.display())
}
