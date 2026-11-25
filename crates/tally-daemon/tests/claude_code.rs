//! Fixture end-to-end for the Claude Code source. Never touches the real `~/.claude`: every run
//! points `projects_dir` at a tempdir.
use rusqlite::Connection;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tally_daemon::claude_code::line::parse_rfc3339_ms;

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/claude_code/projects");
const S1: &str = "__PROJECT__/aaaaaaaa-0000-4000-8000-000000000001.jsonl";
const S2: &str = "__PROJECT__/aaaaaaaa-0000-4000-8000-000000000002.jsonl";
const SENTENCES: [&str; 4] = [
    "explain why the ledger totals look wrong",
    "propose the next small step for the widget parser",
    "Placeholder prompt in a second session",
    "cargo test",
];

fn tally(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tally")).arg("--home").arg(home).args(args).output().unwrap()
}
fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).replace("\r\n", "\n").trim().to_string()
}
fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).to_string()
}
fn run_once(home: &Path) -> (String, String) {
    let o = tally(home, &["run", "--once"]);
    assert!(o.status.success(), "{}", err(&o));
    (out(&o), err(&o))
}

fn replace(hay: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    let (mut r, mut i) = (Vec::with_capacity(hay.len()), 0);
    while i < hay.len() {
        if hay[i..].starts_with(from) {
            r.extend_from_slice(to);
            i += from.len();
        } else {
            r.push(hay[i]);
            i += 1;
        }
    }
    r
}

/// UTC `YYYY-MM-DDTHH:` of the hour containing `ms` (Hinnant's civil_from_days).
fn hour_prefix(ms: i64) -> String {
    let (days, hh) = (ms.div_euclid(86_400_000), ms.rem_euclid(86_400_000) / 3_600_000);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:")
}

struct Env {
    _t: tempfile::TempDir,
    home: PathBuf,
    repo: PathBuf,
    projects: PathBuf,
    /// Replaces the fixture's `2026-11-22T10:` so every fixture time is 85 to 150 minutes old.
    prefix: String,
    cwd_json: String,
}

impl Env {
    fn path(&self, rel: &str) -> PathBuf {
        self.projects.join(rel)
    }
    fn fix(&self, bytes: &[u8]) -> Vec<u8> {
        let b = replace(bytes, b"__CWD__", self.cwd_json.as_bytes());
        replace(&b, b"2026-11-22T10:", self.prefix.as_bytes())
    }
    fn append(&self, rel: &str, text: &str) {
        let mut f = std::fs::OpenOptions::new().append(true).open(self.path(rel)).unwrap();
        f.write_all(&self.fix(text.as_bytes())).unwrap();
    }
    fn copy_fixture(&self) {
        fn walk(from: &Path, to: &Path, env: &Env) {
            std::fs::create_dir_all(to).unwrap();
            for e in std::fs::read_dir(from).unwrap() {
                let p = e.unwrap().path();
                let dst = to.join(p.file_name().unwrap());
                if p.is_dir() {
                    walk(&p, &dst, env);
                } else {
                    std::fs::write(dst, env.fix(&std::fs::read(&p).unwrap())).unwrap();
                }
            }
        }
        walk(Path::new(FIXTURE), &self.projects, self);
    }
    fn db(&self) -> Connection {
        Connection::open(self.home.join("tally.db")).unwrap()
    }
    fn ts(&self, rest: &str) -> i64 {
        parse_rfc3339_ms(&format!("{}{rest}", self.prefix)).unwrap()
    }
    fn count(&self, kind: &str) -> i64 {
        self.db()
            .query_row("SELECT COUNT(*) FROM events WHERE json_extract(json,'$.kind') = ?1", [kind], |r| r.get(0))
            .unwrap()
    }
    fn horizon(&self) -> i64 {
        self.db().query_row("SELECT ts FROM watermarks WHERE source='claude-code'", [], |r| r.get(0)).unwrap()
    }
}

fn git(dir: &Path, args: &[&str]) -> bool {
    Command::new("git").arg("-C").arg(dir).args(args).output().is_ok_and(|o| o.status.success())
}

/// `None` (with a note) when `git` is missing.
fn setup() -> Option<Env> {
    let t = tempfile::tempdir().unwrap();
    let (home, repo, projects) = (t.path().join("home"), t.path().join("repo"), t.path().join("projects"));
    for d in [&home, &repo, &projects] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::write(repo.join("README.md"), "demo widget parser ledger readme\n").unwrap();
    if !(git(&repo, &["init", "-q"])
        && git(&repo, &["add", "README.md"])
        && git(&repo, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "init"]))
    {
        eprintln!("git missing or failing; skipping");
        return None;
    }
    let cwd = repo.to_string_lossy().replace('\\', "/");
    let cwd_json = serde_json::to_string(&cwd).unwrap().trim_matches('"').to_string();
    std::fs::write(home.join("config.toml"), format!("[presence]\nenabled = false\n[heartbeat]
enabled = false
[copilot]\nenabled = false\n[claude_code]\nprojects_dir = '{}'\n", projects.display())).unwrap();
    let now = tally_daemon::source::now_ms();
    let prefix = hour_prefix(now - 90 * 60_000);
    let hour = parse_rfc3339_ms(&format!("{prefix}00:00.000Z")).unwrap();
    assert!(hour <= now - 90 * 60_000 && hour > now - 150 * 60_000, "{prefix}");
    Some(Env { _t: t, home, repo, projects, prefix, cwd_json })
}

/// Steps 1 and 2: live capture on an empty dir, then the fixture arrives as new files.
fn capture_and_read(e: &Env) {
    let (o, _) = run_once(&e.home);
    assert!(o.starts_with("added 0,"), "{o}");
    let started: String = e
        .db()
        .query_row("SELECT value FROM source_state WHERE source='claude-code' AND key='started'", [], |r| r.get(0))
        .unwrap();
    assert!(started.parse::<i64>().unwrap() > 0);
    e.copy_fixture();
    let (_, stderr) = run_once(&e.home);
    assert!(stderr.contains("skipped 2 unreadable lines"), "{stderr}");
    assert!(!stderr.contains("late"), "{stderr}");
}

#[test]
fn fixture_sessions_end_to_end() {
    let Some(e) = setup() else { return };
    capture_and_read(&e);

    // 3. counts by kind
    for (kind, n) in
        [("prompt", 4), ("interrupt", 1), ("tool_decision", 1), ("test_run", 1), ("commit", 1), ("agent_busy", 3), ("agent_output", 5)]
    {
        assert_eq!(e.count(kind), n, "{kind}");
    }
    let total: i64 = e.db().query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0)).unwrap();
    assert_eq!(total, 16);
    let sub: i64 = e.db().query_row("SELECT COUNT(*) FROM events WHERE id LIKE '%a000000000000001%' OR id LIKE '%dddddddd%'", [], |r| r.get(0)).unwrap();
    assert_eq!(sub, 0);
    let (code, init): (i64, String) = e
        .db()
        .query_row("SELECT json_extract(json,'$.exit_code'), json_extract(json,'$.initiator') FROM events WHERE json_extract(json,'$.kind')='test_run'", [], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap();
    assert_eq!((code, init.as_str()), (1, "agent"));
    let by: String = e.db().query_row("SELECT json_extract(json,'$.by') FROM events WHERE json_extract(json,'$.kind')='commit'", [], |r| r.get(0)).unwrap();
    assert_eq!(by, "agent");
    // P1, P2 and session 2's turns, at their prompt timestamps
    let busy: Vec<i64> = {
        let db = e.db();
        let mut st = db.prepare("SELECT ts FROM events WHERE json_extract(json,'$.kind')='agent_busy' ORDER BY ts").unwrap();
        st.query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect()
    };
    assert_eq!(busy, [e.ts("00:05.000Z"), e.ts("00:20.000Z"), e.ts("03:00.000Z")]);

    // 4. project is the repo's unnamed local id
    let id = tally_daemon::project_id(&e.repo).unwrap();
    let wrong: i64 = e
        .db()
        .query_row("SELECT COUNT(*) FROM events WHERE json_extract(json,'$.kind')='prompt' AND json_extract(json,'$.project') IS NOT ?1", [&id], |r| r.get(0))
        .unwrap();
    assert_eq!(wrong, 0);

    // 5. the open P3 turn holds the horizon
    let p3 = e.ts("05:00.000Z");
    assert!(e.horizon() < p3);

    // 6. no text on disk
    for f in std::fs::read_dir(&e.home).unwrap() {
        let p = f.unwrap().path();
        let bytes = std::fs::read(&p).unwrap();
        for s in SENTENCES {
            assert!(!bytes.windows(s.len()).any(|w| w == s.as_bytes()), "{s} in {}", p.display());
        }
    }

    // 7. idempotent
    let (o, _) = run_once(&e.home);
    assert!(o.starts_with("added 0,"), "{o}");

    // 8. the partial last line completes
    e.append(S1, r#"xt","text":"Placeholder rest."}]},"sessionId":"aaaaaaaa-0000-4000-8000-000000000001","cwd":"__CWD__"}
"#);
    let (o, _) = run_once(&e.home);
    assert!(o.starts_with("added 1,"), "{o}");

    // 9. P3 closes
    e.append(S1, r#"{"parentUuid":"bbbbbbbb-0000-4000-8000-000000000018","isSidechain":false,"cwd":"__CWD__","sessionId":"aaaaaaaa-0000-4000-8000-000000000001","type":"system","uuid":"bbbbbbbb-0000-4000-8000-000000000019","timestamp":"2026-11-22T10:06:00.000Z","subtype":"turn_duration","durationMs":60000,"messageCount":2,"isMeta":false}
"#);
    let (o, _) = run_once(&e.home);
    assert!(o.starts_with("added 1,"), "{o}");
    assert_eq!(e.count("agent_busy"), 4);
    assert!(e.horizon() > p3);

    // 10. re-read from scratch: all duplicates
    e.db().execute("UPDATE source_state SET value='0' WHERE key LIKE 'off:%'", []).unwrap();
    let (o, stderr) = run_once(&e.home);
    assert!(o.starts_with("added 0,"), "{o}");
    assert!(!stderr.contains("late"), "{stderr}");
}

#[test]
fn a_rotated_file_is_reread_without_duplicates() {
    let Some(e) = setup() else { return };
    capture_and_read(&e);
    let bytes = std::fs::read(e.path(S2)).unwrap();
    let two: Vec<u8> = bytes.split_inclusive(|b| *b == b'\n').take(2).flatten().copied().collect();
    assert!(two.len() < bytes.len());
    // New uuids, so only the rotation rule (drop ts <= last_ts), not id dedup, keeps these out:
    // they sit under finalized_through and would make the batch late.
    std::fs::write(e.path(S2), replace(&two, b"cccccccc-", b"eeeeeeee-")).unwrap();
    let (o, stderr) = run_once(&e.home);
    assert!(o.starts_with("added 0,"), "{o}");
    assert!(!stderr.contains("late"), "{stderr}");
}
