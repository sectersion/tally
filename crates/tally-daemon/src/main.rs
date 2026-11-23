use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use tally_daemon::config;
use tally_daemon::run::{hooks, run_loop, run_once, sources};
use tally_daemon::source::now_ms;
use tally_daemon::finalize::finalize;
use tally_daemon::status::{status, verify_ledger, VerifyError};
use tally_daemon::store::Store;
use tally_daemon::ingest::ingest_ndjson;
use tally_daemon::project::register_git_project;

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
    ///
    /// Then sets the source's watermark: `--watermark`, else the newest event timestamp in
    /// the file. That default is correct only for a complete, time-ordered file. Sources that
    /// write closed spans (`idle`, `agent_busy`) whose start is earlier than events already
    /// written must pass `--watermark` explicitly.
    Ingest {
        file: PathBuf,
        #[arg(long, default_value = "ingest")]
        source: String,
        /// Promise that this source sends no more events at or before this ms timestamp.
        /// Default: the newest event timestamp in the file, correct only for a complete,
        /// time-ordered file; sources that write closed spans with earlier start timestamps
        /// must pass it explicitly.
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
    /// Poll the sources and finalize every `run.interval_secs` until interrupted.
    Run {
        /// Overrides `run.interval_secs` from config.toml.
        #[arg(long)]
        interval_secs: Option<u64>,
        /// Do one tick, print the result and exit.
        #[arg(long)]
        once: bool,
    },
}

static STOP: AtomicBool = AtomicBool::new(false);

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
    let db = home.join("tally.db");
    if matches!(cli.cmd, Cmd::Ingest { .. } | Cmd::Project { .. } | Cmd::Run { .. }) {
        std::fs::create_dir_all(&home).with_context(|| format!("creating {}", home.display()))?;
    } else if !db.exists() {
        bail!("no database at {}; run `tally ingest` or `tally project add` first", db.display());
    }
    let mut store = Store::open(&db)?;

    match cli.cmd {
        Cmd::Run { interval_secs, once } => {
            let mut cfg = config::load(&home)?;
            if let Some(n) = interval_secs {
                cfg.run.interval_secs = n;
            }
            if once {
                let mut srcs = sources(&cfg, &home, &store);
                let r = run_once(&mut store, &mut srcs, &mut hooks(&cfg, &home), now_ms());
                for e in &r.errors {
                    eprintln!("{e}");
                }
                let (total, prov) = r.finalized.map_or((0, 0), |f| (f.final_total, f.provisional));
                println!("added {}, duplicates {}, final {total}, provisional {prov}", r.added, r.duplicates);
            } else {
                ctrlc::set_handler(|| STOP.store(true, Ordering::Relaxed)).context("installing the Ctrl-C handler")?;
                run_loop(&mut store, &cfg, &home, &STOP);
                println!("stopped");
            }
        }
        Cmd::Ingest { file, source, watermark } => {
            let text = std::fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
            let s = ingest_ndjson(&mut store, &text, &source, watermark)?;
            println!("added {}, duplicates {}", s.added, s.duplicates);
        }
        Cmd::Project { cmd: ProjectCmd::Add { path, name } } => {
            let id = register_git_project(&mut store, &path, name.as_deref())?;
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
