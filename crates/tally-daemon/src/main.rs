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
    /// Commit heads and upload every linked project's published chains to `[sync] server` now (ignores backoff).
    Sync {
        /// Print what each chain would upload (one GET per chain); upload nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Diagnose a ledger that no longer matches a fresh derivation (read-only).
    Repair {
        /// Close the current chain and start a new epoch (not yet available).
        #[arg(long, requires = "reason")]
        new_epoch: bool,
        #[arg(long, value_enum, requires = "new_epoch")]
        reason: Option<tally_daemon::repair::Reason>,
    },
    /// Per-harness log health (files found, last event, parse error rate, format). Always exits 0.
    Doctor {
        #[arg(long)]
        json: bool,
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
    if let Cmd::Repair { new_epoch: true, .. } = &cli.cmd {
        eprintln!("{}", tally_daemon::repair::NEW_EPOCH_UNAVAILABLE);
        std::process::exit(1);
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
        Cmd::Sync { dry_run } => {
            let server = config::load(&home)?.sync.server.ok_or_else(|| anyhow!("no [sync] server in config.toml"))?;
            let tokens = tally_daemon::credentials::load_tokens(&home).context("reading device tokens")?;
            let (commits, chains) = if dry_run {
                (vec![], tally_daemon::sync::dry_run(&mut store, &server, &tokens))
            } else {
                tally_daemon::sync::sync_once(&mut store, &server, &tokens, &mut Default::default(), Some(now_ms()))
            };
            let ids = store.server_ids()?;
            let mut failed = false;
            let mut fail = |p: &str, e: &tally_daemon::sync::SyncError| {
                failed = true;
                let id = ids.get(p).and_then(|v| v.first()).map_or("<id>", String::as_str);
                match e {
                    tally_daemon::sync::SyncError::NotLoggedIn { .. } => eprintln!("{p}: no device token; run tally login --server-id {id}"),
                    tally_daemon::sync::SyncError::Api(tally_daemon::api::ApiError::Unauthorized) => {
                        eprintln!("{p}: device token rejected; run tally login --server-id {id}")
                    }
                    e @ tally_daemon::sync::SyncError::Api(tally_daemon::api::ApiError::Rejected { .. }) => {
                        eprintln!("{p}: {e}; run tally repair")
                    }
                    e => eprintln!("{p}: {e}"),
                }
            };
            for (p, r) in &commits {
                if let Err(e) = r {
                    fail(p, e);
                }
            }
            for (p, r) in &chains {
                match r {
                    Ok(c) if dry_run => println!("{} {}: {} to upload", c.project, c.ruleset, c.uploaded),
                    Ok(c) => println!("{} {}: uploaded {}, server has {}", c.project, c.ruleset, c.uploaded, c.server_count),
                    Err(e) => fail(p, e),
                }
            }
            if failed {
                std::process::exit(1);
            }
        }
        Cmd::Repair { .. } => {
            use tally_daemon::repair::{diagnose, Diagnosis};
            let msg = match diagnose(&store)? {
                Diagnosis::Clean => None,
                Diagnosis::ScoringChanged { index, field, .. } => Some(format!("scoring changed at entry {index} ({field}): uploads are stopped; repair needs plan 10 Task 1b merged and server epoch support; see docs/superpowers/plans/2026-11-23-07-daemon-sync.md Task 9")),
                Diagnosis::Diverged { index, field, .. } => Some(format!("ledger diverged at entry {index} ({field}): uploads are stopped for all projects; a new epoch needs tally repair --new-epoch (not yet available: needs plan 10 Task 1b merged and server epoch support)")),
                Diagnosis::Published(e) => Some(format!("published chains are inconsistent: {e}")),
            };
            if let Some(m) = msg {
                println!("{m}");
                std::process::exit(1);
            }
            println!("ok: ledger matches a fresh derivation");
            // Server counts, when logged in: one GET /head per linked chain.
            if let Some(server) = config::load(&home)?.sync.server {
                let tokens = tally_daemon::credentials::load_tokens(&home).context("reading device tokens")?;
                for (p, r) in tally_daemon::sync::dry_run(&mut store, &server, &tokens) {
                    match r {
                        Ok(c) => println!("{} {}: server has {}", c.project, c.ruleset, c.server_count),
                        Err(e) => eprintln!("{p}: {e}"),
                    }
                }
            }
        }
        Cmd::Doctor { json } => {
            let cfg = config::load(&home)?;
            let configured: Vec<&str> = if cfg.claude_code.enabled { vec!["claude-code"] } else { vec![] };
            let r = tally_daemon::doctor::report(&store, &configured)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&r)?);
            } else {
                for h in &r {
                    println!("{}", tally_daemon::doctor::line(h));
                }
            }
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

/// Stores the token from stdin, then checks it against `[sync] server` if one is configured.
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
    // One GET /head when a server is configured. The token stays stored either way.
    if let Some(url) = config::load(home)?.sync.server {
        let token = tally_daemon::credentials::load_tokens(home).context("reading device tokens")?.remove(server_id).unwrap_or_default();
        match tally_daemon::api::Api::new(&url, &token).head(server_id, tally_scoring::RULESET_VERSION) {
            Ok(_) => {}
            Err(tally_daemon::api::ApiError::Unauthorized) => bail!("device token rejected by {url}; the token is stored, run tally login again with a valid one"),
            Err(e) => eprintln!("warning: could not check the token with {url}: {e}"),
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
