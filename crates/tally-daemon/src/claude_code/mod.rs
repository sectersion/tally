//! The Claude Code session-log source: tails `<projects_dir>/*/*.jsonl` (main sessions only; subagent
//! files under `<session>/subagents/` are never read). Text stays in memory; state keys per file are
//! `off:<rel>` (byte cursor), `last_ts:<rel>` (newest event ts sent), `open:<rel>` (`"<uuid> <ts>"` or `-`).
pub mod features;
pub mod line;
pub mod session;
pub mod tail;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tally_scoring::text::Vocab;
use tally_scoring::Event;

use crate::project::project_for_dir;
use crate::source::{Poll, Source, SourceError, SourceState};
use session::{Ctx, Session};

/// Covers entries whose timestamp is earlier than the moment their line is flushed, and a partial last line.
pub const WRITE_SLACK_MS: i64 = 120_000;
pub const POLL_BUDGET: usize = 4 << 20;
const WARM_UP_BYTES: u64 = 64 << 10;
const VOCAB_TTL: Duration = Duration::from_secs(600);

struct File {
    session: Session,
    /// Session id for `expire`: the file stem until an entry names it.
    sid: String,
    project: Option<String>,
    /// Git root of `project`, to make agent-written paths root-relative.
    root: Option<PathBuf>,
    /// After a rotation: events at or before this ts were already sent and are dropped.
    floor: Option<i64>,
    /// Byte offset the in-memory session has consumed up to. If the saved `off:` differs (the last tick's
    /// state was not persisted, or another process moved it), the session is rebuilt from the saved cursor.
    fed_to: u64,
}

impl File {
    fn new(path: &Path) -> Self {
        let sid = path.file_stem().map_or(String::new(), |s| s.to_string_lossy().into_owned());
        File { session: Session::default(), sid, project: None, root: None, floor: None, fed_to: 0 }
    }
}

pub struct ClaudeCode {
    projects_dir: PathBuf,
    files: BTreeMap<String, File>,
    // ponytail: a cwd that is not (yet) in git stays unmapped until restart.
    projects: HashMap<String, Option<(PathBuf, String)>>,
    vocabs: HashMap<PathBuf, (Instant, Vocab)>,
    vocab_failed: HashSet<PathBuf>,
    /// Bytes per file per poll; `POLL_BUDGET` except in tests.
    budget: usize,
    /// Hashed agent file writes `(file_hash, ts)` since the last `take_agent_writes`; never a path.
    agent_writes: Vec<(String, i64)>,
}

/// `(file_hash, ts)` for each write under `root`; paths outside it are dropped, the path text is not kept.
fn hash_writes(writes: &[(i64, String)], root: &Path, project_id: &str) -> Vec<(String, i64)> {
    let root = crate::project::normalize_path(&root.to_string_lossy());
    writes
        .iter()
        .filter_map(|(ts, p)| {
            let rel = crate::heartbeat::rel_under(&root, &crate::project::normalize_path(p))?;
            Some((crate::heartbeat::file_hash(project_id, &rel), *ts))
        })
        .collect()
}

impl ClaudeCode {
    pub fn new(projects_dir: PathBuf) -> Self {
        ClaudeCode {
            budget: POLL_BUDGET,
            projects_dir,
            files: BTreeMap::new(),
            projects: HashMap::new(),
            vocabs: HashMap::new(),
            vocab_failed: HashSet::new(),
            agent_writes: Vec::new(),
        }
    }

    /// (project id, git root) for a cwd; refreshes the root's vocabulary when missing or stale.
    fn resolve(&mut self, cwd: Option<&str>) -> (Option<String>, Option<PathBuf>) {
        let Some(cwd) = cwd else { return (None, None) };
        let p = self.projects.entry(cwd.to_owned()).or_insert_with(|| project_for_dir(Path::new(cwd)));
        let Some((root, id)) = p.clone() else { return (None, None) };
        if self.vocabs.get(&root).is_none_or(|(t, _)| t.elapsed() > VOCAB_TTL) {
            let v = features::project_vocab(&root).unwrap_or_else(|e| {
                if self.vocab_failed.insert(root.clone()) {
                    eprintln!("source claude-code: vocabulary for {}: {e}", root.display());
                }
                Vocab::default()
            });
            self.vocabs.insert(root.clone(), (Instant::now(), v));
        }
        (Some(id), Some(root))
    }

    /// Feeds lines to the file's session; returns the number of unreadable lines.
    fn feed<'a>(&mut self, rel: &str, lines: impl Iterator<Item = &'a [u8]>, out: &mut Vec<Event>) -> usize {
        let empty = Vocab::default();
        let mut skipped = 0;
        for l in lines {
            let Ok(entries) = line::classify(l) else {
                skipped += 1;
                continue;
            };
            for e in entries {
                let (project, root) = self.resolve(e.meta().cwd.as_deref());
                let vocab = root.as_ref().and_then(|r| self.vocabs.get(r)).map_or(&empty, |v| &v.1);
                let f = self.files.get_mut(rel).expect("file registered before feeding");
                f.sid.clone_from(&e.meta().session);
                f.project.clone_from(&project);
                f.root.clone_from(&root);
                f.session.feed(e, &Ctx { project, vocab }, out);
            }
        }
        skipped
    }

    /// After a restart: replays the last 64 KiB before `off` (events discarded) to rebuild the context tail
    /// and pending tool uses, then restores the open turn from state.
    fn warm_up(&mut self, rel: &str, path: &Path, off: u64, open: Option<String>) {
        let start = off.saturating_sub(WARM_UP_BYTES);
        let mut buf = Vec::new();
        let read = std::fs::File::open(path).and_then(|mut f| {
            f.seek(SeekFrom::Start(start))?;
            f.take(off - start).read_to_end(&mut buf)
        });
        if read.is_ok() {
            let body = match (start, buf.iter().position(|b| *b == b'\n')) {
                (0, _) => &buf[..],
                (_, Some(i)) => &buf[i + 1..],
                (_, None) => &buf[..0],
            };
            let lines = body.split(|b| *b == b'\n').map(|l| l.strip_suffix(b"\r").unwrap_or(l)).filter(|l| !l.is_empty());
            self.feed(rel, lines, &mut Vec::new());
        }
        self.files.get_mut(rel).expect("registered").session.agent_writes.clear(); // warm-up writes are old news
        let s = &mut self.files.get_mut(rel).expect("registered").session;
        match open.as_deref().map(|o| o.split_once(' ')) {
            Some(None) => s.open_turn = None, // "-"
            Some(Some((uuid, ts))) => {
                if let Ok(ts) = ts.parse::<i64>() {
                    s.open_turn = Some((uuid.to_owned(), ts));
                    s.last_seen_ts = s.last_seen_ts.max(ts);
                }
            }
            None => {}
        }
    }
}

/// State encoding of the open turn: `"<uuid> <ts>"`, or `-` when closed.
fn open_str(s: &Session) -> String {
    s.open_turn.as_ref().map_or("-".to_owned(), |(u, t)| format!("{u} {t}"))
}

/// `(rel, path)` of `<dir>/*/*.jsonl`, sorted; a missing dir is empty.
fn session_files(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut v = Vec::new();
    for d in std::fs::read_dir(dir).into_iter().flatten().flatten().filter(|d| d.path().is_dir()) {
        for f in std::fs::read_dir(d.path()).into_iter().flatten().flatten() {
            let p = f.path();
            if p.extension().is_some_and(|e| e == "jsonl") && p.is_file() {
                v.push((format!("{}/{}", d.file_name().to_string_lossy(), f.file_name().to_string_lossy()), p));
            }
        }
    }
    v.sort();
    v
}

impl Source for ClaudeCode {
    fn id(&self) -> &str {
        "claude-code"
    }

    fn poll(&mut self, now_ms: i64, state: &mut dyn SourceState) -> Result<Poll, SourceError> {
        let files = session_files(&self.projects_dir);
        if state.get("started").is_none() {
            // Live capture: history before the first run is not read. Nothing read, so no promise either.
            for (rel, p) in &files {
                let len = std::fs::metadata(p).map_or(0, |m| m.len());
                state.set(&format!("off:{rel}"), &len.to_string());
            }
            state.set("started", &now_ms.to_string());
            return Ok(Poll { events: vec![], watermark: None });
        }
        self.files.retain(|k, _| files.iter().any(|(rel, _)| rel == k));
        let (mut events, mut skipped) = (Vec::new(), 0);
        // A file with complete lines left unread (budget hit or read error) holds the watermark below the
        // newest timestamp it has shown, minus the write slack (unread lines may be a little older), and below
        // a persisted open turn (`open_ts`, for a file not registered in this process yet); one that has shown
        // no timestamp withholds the watermark for this poll.
        let mut cap: Option<i64> = Some(i64::MAX);
        let mut hold = |f: Option<&File>, last_ts: Option<i64>, open_ts: Option<i64>| {
            let seen = f.map(|f| f.session.last_seen_ts).filter(|t| *t > 0).or(last_ts);
            let open = open_ts.map_or(i64::MAX, |t| t.saturating_sub(1));
            cap = cap.and_then(|c| seen.map(|t| c.min(t.saturating_sub(1 + WRITE_SLACK_MS)).min(open)));
        };
        for (rel, path) in &files {
            let (off_key, last_key, open_key) = (format!("off:{rel}"), format!("last_ts:{rel}"), format!("open:{rel}"));
            let off: u64 = state.get(&off_key).and_then(|v| v.parse().ok()).unwrap_or(0);
            let last_ts: Option<i64> = state.get(&last_key).and_then(|v| v.parse().ok());
            let chunk = match tail::read_lines(path, off, self.budget) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("source claude-code: {}: {e}", path.display());
                    let open_ts = state.get(&open_key).and_then(|o| o.split_once(' ')?.1.parse().ok());
                    hold(self.files.get(rel), last_ts, open_ts);
                    continue;
                }
            };
            if chunk.rotated {
                // The open turn's agent_busy was not sent yet; its ts is the prompt ts, so keep the floor below it
                // (re-sent events dedupe by id, and the open turn held the watermark below its start).
                let open_ts: Option<i64> = state.get(&open_key).and_then(|o| o.split_once(' ')?.1.parse().ok());
                let floor = last_ts.map(|l| open_ts.map_or(l, |t| l.min(t - 1)));
                self.files.insert(rel.clone(), File { floor, ..File::new(path) });
            } else if self.files.get(rel).is_none_or(|f| {
                // Saved state differs from memory (last tick not persisted, e.g. an expiry with no new lines).
                f.fed_to != off || open_str(&f.session) != state.get(&open_key).unwrap_or_else(|| "-".into())
            }) {
                let floor = self.files.get(rel).and_then(|f| f.floor);
                self.files.insert(rel.clone(), File { floor, ..File::new(path) });
                if off > 0 {
                    self.warm_up(rel, path, off, state.get(&open_key));
                }
            }
            let mut out = Vec::new();
            skipped += self.feed(rel, chunk.lines.iter().map(Vec::as_slice), &mut out);
            let f = self.files.get_mut(rel).expect("registered");
            f.fed_to = chunk.end;
            if chunk.budget_hit {
                // The session's own limit() covers its open turn. No expiry: the rest of the turn may be unread.
                hold(Some(f), last_ts, None);
            } else {
                f.session.expire(now_ms, &f.sid, f.project.clone(), &mut out);
            }
            let writes = std::mem::take(&mut f.session.agent_writes);
            if let (Some(root), Some(id)) = (&f.root, &f.project) {
                self.agent_writes.extend(hash_writes(&writes, root, id));
            }
            if let Some(floor) = f.floor {
                out.retain(|e| e.ts > floor);
            }
            if let Some(m) = out.iter().map(|e| e.ts).chain(last_ts).max() {
                state.set(&last_key, &m.to_string());
            }
            state.set(&off_key, &chunk.end.to_string());
            state.set(&open_key, &open_str(&f.session));
            events.extend(out);
        }
        if skipped > 0 {
            eprintln!("source claude-code: skipped {skipped} unreadable lines");
        }
        let w = self.files.values().map(|f| f.session.limit()).fold(now_ms - WRITE_SLACK_MS, i64::min);
        Ok(Poll { events, watermark: cap.map(|c| c.min(w)) })
    }

    fn take_agent_writes(&mut self) -> Vec<(String, i64)> {
        std::mem::take(&mut self.agent_writes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::run_once;
    use crate::store::Store;
    use line::parse_rfc3339_ms;
    use std::io::Write;

    fn ts(i: usize) -> String {
        format!("2026-11-22T10:{:02}:{:02}.000Z", i / 60, i % 60)
    }

    #[test]
    fn hash_writes_hashes_root_relative_paths() {
        let want = crate::heartbeat::file_hash("p", "src/a.rs");
        let w = [
            (1, r"C:\tallyproj\src\a.rs".to_owned()),
            (2, "C:/tallyproj/src/a.rs".to_owned()),
            (3, "D:/other/b.rs".to_owned()),
            (4, "C:/tallyproj2/src/a.rs".to_owned()),
        ];
        let got = hash_writes(&w, Path::new("C:/tallyproj"), "p");
        assert_eq!(got, [(want.clone(), 1), (want.clone(), 2)]);
        assert!(!format!("{got:?}").contains("src/a.rs"));
        // production roots are canonicalized: verbatim prefix and backslashes
        assert_eq!(hash_writes(&w, Path::new(r"\\?\C:\tallyproj"), "p"), got);
    }

    #[test]
    fn claude_edit_suppresses_the_matching_heartbeat() {
        use crate::heartbeat::{map_heartbeats, server::Received, Heartbeat};
        let d = tempfile::tempdir().unwrap();
        let repo = std::fs::canonicalize(d.path()).unwrap().join("repo");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::create_dir(d.path().join("p")).unwrap();
        let file = crate::project::normalize_path(&repo.join("src/a.rs").to_string_lossy());
        let cwd = crate::project::normalize_path(&repo.to_string_lossy());
        let t0 = ts(0);
        let line = format!(
            r#"{{"type":"assistant","uuid":"a1","sessionId":"s","timestamp":"{t0}","cwd":"{cwd}","message":{{"content":[{{"type":"tool_use","id":"t1","name":"Edit","input":{{"file_path":"{file}"}}}}]}}}}"#
        );
        std::fs::write(d.path().join("p/s.jsonl"), line + "\n").unwrap();
        let at = parse_rfc3339_ms(&t0).unwrap();
        let mut src = ClaudeCode::new(d.path().to_path_buf());
        let mut st = BTreeMap::from([("started".to_owned(), "1".to_owned())]);
        src.poll(at + 3_600_000, &mut st).unwrap();
        let w = src.take_agent_writes();
        assert_eq!(w.len(), 1);
        assert!(src.take_agent_writes().is_empty());
        let hb: Heartbeat = serde_json::from_value(serde_json::json!({
            "entity": repo.join("src/a.rs").to_string_lossy(), "type": "file", "time": (at + 2_000) as f64 / 1000.0, "is_write": true
        }))
        .unwrap();
        let batch = [Received { hb, received_ms: at + 2_000 }];
        let m = map_heartbeats(&batch, &project_for_dir, &|h, from, to| w.iter().any(|(wh, t)| wh == h && *t >= from && *t <= to), 0, &mut BTreeMap::new());
        assert_eq!((m.stats.dropped_agent, m.events.len()), (1, 0));
    }

    #[test]
    fn an_expiry_whose_state_was_not_saved_is_redone() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("p")).unwrap();
        let t0 = ts(0);
        let line = format!(r#"{{"type":"user","uuid":"p1","sessionId":"s","timestamp":"{t0}","origin":{{"kind":"human"}},"message":{{"content":"go"}}}}"#);
        std::fs::write(d.path().join("p/s.jsonl"), line + "\n").unwrap();
        let start = parse_rfc3339_ms(&t0).unwrap();
        let mut src = ClaudeCode::new(d.path().to_path_buf());
        let mut st = BTreeMap::from([("started".to_owned(), "1".to_owned())]);
        assert_eq!(src.poll(start + 3_600_000, &mut st).unwrap().events.len(), 1); // the prompt
        // No new lines; the stale turn expires, but this tick's store write fails.
        let late = start + session::OPEN_SPAN_MAX_MS + 1;
        assert_eq!(src.poll(late, &mut st.clone()).unwrap().events.len(), 1);
        let p = src.poll(late, &mut st).unwrap();
        let ids: Vec<_> = p.events.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["claude:s:p1:agent_busy"]);
    }

    #[test]
    fn a_rotation_keeps_the_busy_span_of_the_turn_open_at_rotation() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("p")).unwrap();
        let f = d.path().join("p/s.jsonl");
        let head = |ty: &str, u: &str, i: usize| {
            let t = ts(i);
            format!(r#"{{"type":"{ty}","uuid":"{u}","sessionId":"s","timestamp":"{t}""#)
        };
        let p = head("user", "p1", 0) + r#","origin":{"kind":"human"},"message":{"content":"go"}}"#;
        let a = head("assistant", "a1", 1) + r#","message":{"content":[{"type":"text","text":"working"}]}}"#;
        let end = head("system", "d1", 3) + r#","subtype":"turn_duration"}"#;
        std::fs::write(&f, [p.as_str(), a.as_str(), &"x".repeat(500)].join("
") + "
").unwrap();
        let now = parse_rfc3339_ms(&ts(0)).unwrap() + 30_000;
        let mut src = ClaudeCode::new(d.path().to_path_buf());
        let mut st = BTreeMap::from([("started".to_owned(), "1".to_owned())]);
        src.poll(now, &mut st).unwrap();
        assert_eq!(src.poll(now, &mut st).unwrap().events.len(), 0);
        // The file shrinks (rewritten), still holding the open turn, and now ends it.
        std::fs::write(&f, [p, a, end].join("
") + "
").unwrap();
        let ids: Vec<_> = src.poll(now, &mut st).unwrap().events.into_iter().map(|e| e.id).collect();
        assert!(ids.contains(&"claude:s:p1:agent_busy".to_owned()), "{ids:?}");
    }

    #[test]
    fn a_tick_whose_state_was_not_saved_is_replayed_from_the_saved_cursor() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("p")).unwrap();
        let f = d.path().join("p/s.jsonl");
        let head = |ty: &str, u: &str, i: usize| {
            let t = ts(i);
            format!(r#"{{"type":"{ty}","uuid":"{u}","sessionId":"s","timestamp":"{t}""#)
        };
        let a = [
            head("user", "p1", 0) + r#","origin":{"kind":"human"},"message":{"content":"go"}}"#,
            head("assistant", "a1", 1)
                + r#","message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"cargo test"}}]}}"#,
        ];
        let b = [
            head("user", "r1", 2) + r#","message":{"content":[{"type":"tool_result","tool_use_id":"t1","is_error":true,"content":"x"}]}}"#,
            head("system", "d1", 3) + r#","subtype":"turn_duration"}"#,
        ];
        std::fs::write(&f, a.join("\n") + "\n").unwrap();
        let now = parse_rfc3339_ms(&ts(0)).unwrap() + 3_600_000;
        let mut src = ClaudeCode::new(d.path().to_path_buf());
        let mut st = BTreeMap::from([("started".to_owned(), "1".to_owned())]);
        src.poll(now, &mut st).unwrap();
        std::fs::OpenOptions::new().append(true).open(&f).unwrap().write_all((b.join("\n") + "\n").as_bytes()).unwrap();
        // This tick's store write fails: its events and state are discarded.
        let lost = src.poll(now, &mut st.clone()).unwrap();
        assert_eq!(lost.events.len(), 2);
        let p = src.poll(now, &mut st).unwrap();
        let ids: Vec<_> = p.events.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["claude:s:a1:test_run", "claude:s:p1:agent_busy"]);
        assert!(matches!(p.events[1].kind, tally_scoring::Kind::AgentBusy { end_ts } if end_ts == parse_rfc3339_ms(&ts(3)).unwrap()));
    }

    #[test]
    fn a_backlog_over_the_budget_holds_the_watermark_until_it_is_read() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("p")).unwrap();
        let n = 200;
        let body: String = (0..n)
            .map(|i| {
                let t = ts(i);
                let x = "x".repeat(200);
                format!(
                    r#"{{"type":"assistant","uuid":"u{i}","sessionId":"s","timestamp":"{t}","message":{{"content":[{{"type":"text","text":"{x}"}}]}}}}"#
                ) + "\n"
            })
            .collect();
        std::fs::write(d.path().join("p/s.jsonl"), body).unwrap();
        let mut src = ClaudeCode::new(d.path().to_path_buf());
        src.budget = 4096;
        let mut srcs: Vec<Box<dyn Source>> = vec![Box::new(src)];
        let mut store = Store::open_in_memory().unwrap();
        store.set_state("claude-code", &BTreeMap::from([("started".to_owned(), "1".to_owned())])).unwrap();
        let now = parse_rfc3339_ms(&ts(n)).unwrap() + 3_600_000;
        let mut polls = 0;
        loop {
            polls += 1;
            let r = run_once(&mut store, &mut srcs, &mut [], now);
            assert!(r.errors.is_empty(), "poll {polls}: {:?}", r.errors);
            let seen = store.events().unwrap().len();
            let h = store.horizon().unwrap();
            if seen == n {
                assert_eq!(h, Some(now - WRITE_SLACK_MS));
                break;
            }
            let next_unread = parse_rfc3339_ms(&ts(seen)).unwrap();
            assert!(h.is_none_or(|h| h < next_unread), "poll {polls}: horizon {h:?} passes unread data");
            assert!(polls < 100);
        }
        assert!(polls > 5, "{polls}");
    }

    /// Makes `p` unopenable while the guard lives; `None` if the platform cannot (e.g. running as root).
    fn lock(p: &Path) -> Option<Box<dyn std::any::Any>> {
        #[cfg(windows)]
        let g = {
            use std::os::windows::fs::OpenOptionsExt;
            std::fs::OpenOptions::new().read(true).share_mode(0).open(p).ok().map(|f| Box::new(f) as Box<dyn std::any::Any>)
        };
        #[cfg(unix)]
        let g = {
            use std::os::unix::fs::PermissionsExt;
            struct Restore(PathBuf);
            impl Drop for Restore {
                fn drop(&mut self) {
                    let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o644));
                }
            }
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o000)).unwrap();
            Some(Box::new(Restore(p.to_path_buf())) as Box<dyn std::any::Any>)
        };
        std::fs::File::open(p).is_err().then_some(g).flatten()
    }

    #[test]
    fn a_read_error_holds_the_watermark() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("p")).unwrap();
        let f = d.path().join("p/s.jsonl");
        let line = |i: usize| {
            let t = ts(i);
            format!(r#"{{"type":"assistant","uuid":"u{i}","sessionId":"s","timestamp":"{t}","message":{{"content":[{{"type":"text","text":"x"}}]}}}}"#) + "\n"
        };
        std::fs::write(&f, line(0) + &line(1)).unwrap();
        let now = parse_rfc3339_ms(&ts(2)).unwrap() + 3_600_000;
        let mut st = BTreeMap::from([("started".to_owned(), "1".to_owned())]);
        let mut src = ClaudeCode::new(d.path().to_path_buf());
        assert_eq!(src.poll(now, &mut st).unwrap().watermark, Some(now - WRITE_SLACK_MS));
        std::fs::OpenOptions::new().append(true).open(&f).unwrap().write_all(line(2).as_bytes()).unwrap();
        let Some(_g) = lock(&f) else {
            eprintln!("cannot make a file unreadable here; skipping");
            return;
        };
        let p = src.poll(now, &mut st).unwrap();
        let at = |i: usize| parse_rfc3339_ms(&ts(i)).unwrap();
        assert_eq!((p.events.len(), p.watermark), (0, Some(at(1) - 1 - WRITE_SLACK_MS)));
        // a fresh process: no session yet, so last_ts (u1's ts) caps it
        assert_eq!(ClaudeCode::new(d.path().to_path_buf()).poll(now, &mut st).unwrap().watermark, p.watermark);
        // ... and a persisted open turn older than last_ts - slack caps it lower
        st.insert("last_ts:p/s.jsonl".into(), at(1000).to_string());
        st.insert("open:p/s.jsonl".into(), format!("u0 {}", at(0)));
        assert_eq!(ClaudeCode::new(d.path().to_path_buf()).poll(now, &mut st).unwrap().watermark, Some(at(0) - 1));
        st.insert("open:p/s.jsonl".into(), "-".into());
        assert_eq!(
            ClaudeCode::new(d.path().to_path_buf()).poll(now, &mut st).unwrap().watermark,
            Some(at(1000) - 1 - WRITE_SLACK_MS)
        );
        // nothing seen at all: no promise
        st.remove("last_ts:p/s.jsonl");
        assert_eq!(ClaudeCode::new(d.path().to_path_buf()).poll(now, &mut st).unwrap().watermark, None);
    }
}
