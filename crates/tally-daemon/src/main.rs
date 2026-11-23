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
use tally_daemon::project::{project_for_dir, register_git_project_matched};
use tally_daemon::testrun::test_run_event;

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
    /// Write per-project chains and badges to the data branch and push (`publish.enabled` is ignored).
    Publish {
        /// Show what would be written; store nothing and push nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Store a project's device token (read from stdin) in the data dir; the token is never printed.
    ///
    /// With `--server`, also writes `[sync] enabled = true` into config.toml unless it already has a `[sync]` table.
    Login {
        #[arg(long)]
        server_id: String,
        #[arg(long)]
        server: Option<String>,
    },
    /// Remove a project's device token.
    Logout {
        #[arg(long)]
        server_id: String,
    },
    Status,
    Verify,
    /// Check downloaded published chain files; needs no home or database. Exits 1 if any fail.
    VerifyPublished {
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },
    /// Poll the sources and finalize every `run.interval_secs` until interrupted.
    Run {
        /// Overrides `run.interval_secs` from config.toml.
        #[arg(long)]
        interval_secs: Option<u64>,
        /// Do one tick, print the result and exit.
        #[arg(long)]
        once: bool,
    },
    /// Hooks that other tools and shells call.
    ///
    /// No watermark is set for source `shell`: a hook event is stamped at completion with the
    /// current time, and every other source holds its watermark at or below the present, so the
    /// event is essentially never at or below a finalized point (a daemon poll in the millisecond
    /// between the child exiting and the append could race; the hook then warns that the run
    /// was not recorded).
    Hook {
        #[command(subcommand)]
        cmd: HookCmd,
    },
}

#[derive(Subcommand)]
enum HookCmd {
    /// Run a program and record it as a user test run (only a hash of the command is stored).
    ///
    /// Runs the program with inherited stdio and exits with its exit code. Add an alias:
    ///
    /// PowerShell: function tt { tally hook test-run -- @args }
    ///
    /// bash/zsh:   alias tt='tally hook test-run --'
    ///
    /// then use it as `tt cargo test`.
    ///
    /// When the environment variable CLAUDECODE is set (Claude Code sets it in its shells), the
    /// program still runs and its exit code is passed through, but nothing is recorded: the
    /// agent's test runs are not the user's. A human using this inside a Claude Code terminal is
    /// not credited either.
    TestRun {
        #[arg(last = true, required = true)]
        args: Vec<String>,
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
    /// Store the server's project id for a registered repository.
    Link {
        path: PathBuf,
        #[arg(long)]
        server_id: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    if let Cmd::VerifyPublished { files } = &cli.cmd {
        let mut failed = false;
        for f in files {
            let text = std::fs::read_to_string(f).with_context(|| format!("reading {}", f.display()))?;
            match tally_daemon::published::verify_published_json(&text) {
                Ok(c) => {
                    let h = tally_daemon::published::hours_message(c.verified_s);
                    println!("ok: {}: {} entries, project {}, ruleset {}, verified_hours {}", f.display(), c.entries,
                        c.project.as_deref().unwrap_or("none"), c.ruleset.as_deref().unwrap_or("none"), h.trim_end_matches(" h"));
                }
                Err(e) => {
                    println!("{}: {e}", f.display());
                    failed = true;
                }
            }
        }
        std::process::exit(i32::from(failed));
    }
    let home = match cli.home {
        Some(h) => h,
        None => dirs::data_local_dir().ok_or_else(|| anyhow!("no local data directory; pass --home"))?.join("tally"),
    };
    let db = home.join("tally.db");
    if let Cmd::Hook { cmd: HookCmd::TestRun { args } } = &cli.cmd {
        std::process::exit(test_run(&home, &db, args));
    }
    if let Cmd::Login { server_id, server } = &cli.cmd {
        return login(&home, server_id, server.as_deref());
    }
    if let Cmd::Logout { server_id } = &cli.cmd {
        let gone = tally_daemon::credentials::delete_token(&home, server_id).context("updating the token file")?;
        println!("{}", if gone { "logged out" } else { "not logged in" });
        return Ok(());
    }
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
            cfg.validate().map_err(|e| anyhow!(e))?;
            if cfg.presence.required() && !tally_daemon::presence::supported() {
                bail!("presence.require is true but presence is not supported on this platform; set [presence] require = false in config.toml");
            }
            if once {
                let mut srcs = sources(&cfg, &home, &mut store)?;
                let r = run_once(&mut store, &mut srcs, &mut hooks(&cfg, &home), now_ms());
                for e in &r.errors {
                    eprintln!("{e}");
                }
                let (skipped, total, prov) = r.finalized.map_or((0, 0, 0), |f| (f.skipped, f.final_total, f.provisional));
                println!("added {}, duplicates {}, skipped {skipped}, final {total}, provisional {prov}", r.added, r.duplicates);
            } else {
                ctrlc::set_handler(|| STOP.store(true, Ordering::Relaxed)).context("installing the Ctrl-C handler")?;
                run_loop(&mut store, &cfg, &home, &STOP)?;
                println!("stopped");
            }
        }
        Cmd::Ingest { file, source, watermark } => {
            let text = std::fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
            let s = ingest_ndjson(&mut store, &text, &source, watermark)?;
            println!("added {}, duplicates {}", s.added, s.duplicates);
        }
        Cmd::Hook { .. } | Cmd::VerifyPublished { .. } | Cmd::Login { .. } | Cmd::Logout { .. } => {
            unreachable!("handled before the store is opened")
        }
        Cmd::Project { cmd: ProjectCmd::Add { path, name } } => {
            if let Some(n) = name.as_deref().filter(|n| !tally_daemon::published::valid_public_name(n)) {
                bail!("invalid project name {n:?}; use owner/repo (letters, digits, '.', '_', '-')");
            }
            let (id, adopted) = register_git_project_matched(&mut store, &path, name.as_deref())?;
            if let Some((name, via)) = adopted {
                eprintln!("matched {name} via {via}");
            }
            println!("{id}");
        }
        Cmd::Project { cmd: ProjectCmd::Link { path, server_id } } => {
            println!("{}", tally_daemon::project::link_git_project(&mut store, &path, &server_id)?);
        }
        Cmd::Finalize => {
            let s = finalize(&mut store)?;
            println!("appended {}, skipped {}, final {}, provisional {}", s.appended, s.skipped, s.final_total, s.provisional);
        }
        Cmd::Publish { dry_run } => {
            let cfg = config::load(&home)?;
            let r = tally_daemon::publish::publish_once(&mut store, &cfg.publish, &home, dry_run)?;
            print!("{}", r.render_text(dry_run));
        }
        Cmd::Status => {
            let s = status(&store)?;
            println!("events: {}", s.events);
            println!("final_claims: {}", s.final_claims);
            println!("provisional_claims: {}", s.provisional_claims);
            println!("verified_s: {}", s.verified_s);
            println!("unverified_s: {}", s.unverified_s);
            println!("skipped_claims: {}", s.skipped_claims);
            println!("horizon: {}", s.horizon.map_or("none".to_string(), |h| h.to_string()));
            println!("finalized_through: {}", s.finalized_through.map_or("none".to_string(), |t| t.to_string()));
            println!("last_hash: {}", s.last_hash);
            if s.blocked_on_presence {
                println!("blocked: presence required but no presence watermark");
            }
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

/// Stores the token from stdin. No network call here (the server is checked by the sync loop).
fn login(home: &std::path::Path, server_id: &str, server: Option<&str>) -> Result<()> {
    if !tally_daemon::project::valid_server_id(server_id) {
        bail!("invalid server id; use 1 to 64 letters, digits, '_' or '-'");
    }
    std::fs::create_dir_all(home).with_context(|| format!("creating {}", home.display()))?;
    // Validate the config (and any existing [sync] server) before storing a secret.
    config::load(home)?;
    eprint!("paste device token: ");
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).context("reading the token")?;
    tally_daemon::credentials::save_token(home, server_id, &line).map_err(|e| anyhow!("token not stored: {e}"))?;
    if let Some(url) = server {
        let p = home.join("config.toml");
        let existing = std::fs::read_to_string(&p).unwrap_or_default();
        if existing.lines().any(|l| l.trim() == "[sync]") {
            println!("edit [sync] in {}", p.display());
        } else {
            let text = format!("{existing}{}[sync]\nenabled = true\nserver = {}\n", if existing.is_empty() || existing.ends_with('\n') { "" } else { "\n" }, toml::Value::String(url.to_string()));
            std::fs::write(&p, &text).with_context(|| format!("writing {}", p.display()))?;
            if let Err(e) = config::load(home) {
                std::fs::write(&p, &existing)?; // restore: the url was not acceptable
                bail!("{e}");
            }
        }
    }
    println!("logged in");
    Ok(())
}

/// Runs the program, then records the event; a store failure only warns. Returns the exit code.
fn test_run(home: &std::path::Path, db: &std::path::Path, args: &[String]) -> i32 {
    let status = match std::process::Command::new(&args[0]).args(&args[1..]).status() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("tally: cannot run {}: {e}", args[0]);
            return 127;
        }
    };
    let code = status.code().unwrap_or(1);
    if std::env::var_os("CLAUDECODE").is_some() {
        eprintln!("tally: running inside Claude Code; test run not recorded as a user action");
        return code;
    }
    let project = std::env::current_dir().ok().and_then(|d| project_for_dir(&d)).map(|(_, id)| id);
    let ev = test_run_event(args, code, now_ms(), project, std::process::id());
    let recorded = std::fs::create_dir_all(home)
        .map_err(anyhow::Error::from)
        .and_then(|_| Ok(Store::open(db)?.append_events(&[ev])?));
    if let Err(e) = recorded {
        eprintln!("tally: warning: test run not recorded: {e}");
    }
    code
}
