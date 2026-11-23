use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tally_daemon::finalize::finalize;
use tally_daemon::status::{status, verify_ledger, VerifyError};
use tally_daemon::store::Store;
use tally_daemon::{find_git_root, project_id};
use tally_scoring::Event;

#[derive(Parser)]
#[command(name = "tally")]
struct Cli {
    /// Data directory (default: the local data dir, `tally` inside it).
    #[arg(long, global = true, env = "TALLY_HOME")]
    home: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Read one JSON event per line from FILE into the store.
    Ingest {
        file: PathBuf,
        #[arg(long, default_value = "ingest")]
        source: String,
        /// Defaults to the newest event timestamp in the file.
        #[arg(long)]
        watermark: Option<i64>,
    },
    Project {
        #[command(subcommand)]
        cmd: ProjectCmd,
    },
    Finalize,
    Status,
    Verify,
}

#[derive(Subcommand)]
enum ProjectCmd {
    Add {
        path: PathBuf,
        #[arg(long)]
        name: Option<String>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let home = match cli.home {
        Some(h) => h,
        None => dirs::data_local_dir().ok_or_else(|| anyhow!("no local data directory; pass --home"))?.join("tally"),
    };
    std::fs::create_dir_all(&home).with_context(|| format!("creating {}", home.display()))?;
    let mut store = Store::open(&home.join("tally.db"))?;

    match cli.cmd {
        Cmd::Ingest { file, source, watermark } => {
            let text = std::fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
            let mut events = Vec::new();
            for (i, line) in text.lines().enumerate() {
                if line.trim().is_empty() {
                    continue;
                }
                events.push(serde_json::from_str::<Event>(line).with_context(|| format!("line {}", i + 1))?);
            }
            let mark = watermark.or_else(|| events.iter().map(|e| e.ts).max());
            let s = store.append_events(&events)?;
            if let Some(ms) = mark {
                store.set_watermark(&source, ms)?;
            }
            println!("added {}, duplicates {}", s.added, s.duplicates);
        }
        Cmd::Project { cmd: ProjectCmd::Add { path, name } } => {
            let root = find_git_root(&path).ok_or_else(|| anyhow!("not inside a git repository"))?;
            let id = project_id(&root)?;
            store.register_project(&id, &root.to_string_lossy(), name.as_deref())?;
            println!("{id}");
        }
        Cmd::Finalize => {
            let s = finalize(&mut store)?;
            println!("appended {}, final {}, provisional {}", s.appended, s.final_total, s.provisional);
        }
        Cmd::Status => {
            let s = status(&store)?;
            println!("events: {}", s.events);
            println!("final_claims: {}", s.final_claims);
            println!("provisional_claims: {}", s.provisional_claims);
            println!("verified_s: {}", s.verified_s);
            println!("unverified_s: {}", s.unverified_s);
            println!("horizon: {}", s.horizon.map_or("none".to_string(), |h| h.to_string()));
            println!("finalized_through: {}", s.finalized_through.map_or("none".to_string(), |t| t.to_string()));
            println!("last_hash: {}", s.last_hash);
        }
        Cmd::Verify => match verify_ledger(&store) {
            Ok(n) => println!("ok: {n} entries"),
            Err(VerifyError::Chain { index, reason }) => {
                println!("FAIL at entry {index}: {reason}");
                std::process::exit(1);
            }
            Err(e) => return Err(e.into()),
        },
    }
    Ok(())
}
