use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "tally-evidence", about = "Evidence tooling for the Tally pitch. Never shipped to users.", version)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Compare a Hackatime-style estimate with Tally's scoring for one session (NDJSON events, as `tally ingest` reads).
    Replay {
        events: PathBuf,
        /// A real Hackatime heartbeat export (NDJSON `{"ts": <ms>}`); preferred over the synthetic derivation.
        #[arg(long)]
        heartbeats: Option<PathBuf>,
        #[arg(long, default_value = "Hackatime-style vs Tally")]
        title: String,
        #[arg(long)]
        json: bool,
    },
    /// Write a synthetic Claude Code session log (the resource harness's load) into a projects directory.
    Load {
        /// The `projects` directory the adapter reads.
        dir: PathBuf,
        /// A git repository to record as the session's working directory.
        #[arg(long, default_value = ".")]
        cwd: PathBuf,
        #[arg(long, default_value_t = 8)]
        hours: u32,
        /// `real-time` or `accelerated:N`.
        #[arg(long, default_value = "real-time")]
        load: String,
    },
    /// Spawn `tally run`, apply the load, sample ITS memory and CPU, and write the numbers file.
    Resources {
        #[arg(long)]
        tally: PathBuf,
        /// Describe the machine (make, model, year, CPU, RAM, Windows build). Required: numbers without it are not published.
        #[arg(long, default_value = "")]
        machine: String,
        #[arg(long, default_value_t = 60)]
        duration_min: u64,
        /// `real-time` or `accelerated:N` (accelerated runs are labelled as such and never count as the published figure).
        #[arg(long, default_value = "real-time")]
        load: String,
        #[arg(long)]
        out: PathBuf,
        /// Turn presence off in the harness config (CI runners have no idle source).
        #[arg(long)]
        no_presence: bool,
        /// Record whether the laptop was plugged in (`--plugged`) or on battery (`--battery`).
        #[arg(long, conflicts_with = "battery")]
        plugged: bool,
        #[arg(long)]
        battery: bool,
    },
    /// Score every threat-model attack and an honest control; print or write the Markdown report.
    Redteam {
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

fn parse_load(s: &str) -> Result<u64> {
    match s.strip_prefix("accelerated:") {
        Some(n) => n.parse::<u64>().ok().filter(|n| *n >= 1).ok_or_else(|| anyhow::anyhow!("--load accelerated:N needs a whole number N >= 1")),
        None if s == "real-time" => Ok(1),
        None => anyhow::bail!("--load is `real-time` or `accelerated:N`"),
    }
}

fn read_events(path: &std::path::Path) -> Result<Vec<tally_scoring::Event>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    text.trim_start_matches('\u{feff}')
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
        .map(|(i, l)| serde_json::from_str(l).with_context(|| format!("{} line {}", path.display(), i + 1)))
        .collect()
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Replay { events, heartbeats, title, json } => {
            let ev = read_events(&events)?;
            let hb = heartbeats
                .map(|p| tally_evidence::hackatime::from_heartbeat_file(&std::fs::read_to_string(&p).with_context(|| format!("reading {}", p.display()))?))
                .transpose()?;
            let r = tally_evidence::replay::compare(&ev, hb);
            println!("{}", if json { tally_evidence::replay::render_json(&r) } else { tally_evidence::replay::render_markdown(&r, &title) });
        }
        Cmd::Load { dir, cwd, hours, load } => {
            let mut plan = tally_evidence::load::LoadPlan::default_for(cwd);
            plan.hours = hours;
            plan.speedup = parse_load(&load)?;
            let p = tally_evidence::load::write_session_log(&dir, &plan, &mut tally_evidence::load::RealClock)?;
            println!("wrote {}", p.display());
        }
        Cmd::Resources { tally, machine, duration_min, load, out, no_presence, plugged, battery } => {
            let o = tally_evidence::resources::Options {
                tally,
                machine,
                duration: std::time::Duration::from_secs(duration_min * 60),
                speedup: parse_load(&load)?,
                out: out.clone(),
                no_presence,
                plugged: if plugged { Some(true) } else if battery { Some(false) } else { None },
            };
            let a = tally_evidence::resources::run(&o)?;
            println!("wrote {} (active: RSS p95 {:.1} MB, CPU p95 {:.2}%)", out.display(), a.rss_p95 as f64 / 1e6, f64::from(a.cpu_p95_x100) / 100.0);
        }
        Cmd::Redteam { out } => {
            let commit = std::process::Command::new("git")
                .args(["log", "-1", "--format=%h", "--", "crates/tally-scoring"])
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "unknown".into());
            let md = tally_evidence::redteam::render(&commit);
            match out {
                Some(p) => std::fs::write(&p, md).with_context(|| format!("writing {}", p.display()))?,
                None => print!("{md}"),
            }
        }
    }
    Ok(())
}
