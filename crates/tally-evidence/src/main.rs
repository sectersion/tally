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
    /// Score every threat-model attack and an honest control; print or write the Markdown report.
    Redteam {
        #[arg(long)]
        out: Option<PathBuf>,
    },
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
