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
    /// After a rotation: events at or before this ts were already sent and are dropped.
    floor: Option<i64>,
}

impl File {
    fn new(path: &Path) -> Self {
        let sid = path.file_stem().map_or(String::new(), |s| s.to_string_lossy().into_owned());
        File { session: Session::default(), sid, project: None, floor: None }
    }
}

pub struct ClaudeCode {
    projects_dir: PathBuf,
    files: BTreeMap<String, File>,
    // ponytail: a cwd that is not (yet) in git stays unmapped until restart.
    projects: HashMap<String, Option<(PathBuf, String)>>,
    vocabs: HashMap<PathBuf, (Instant, Vocab)>,
    vocab_failed: HashSet<PathBuf>,
}

impl ClaudeCode {
    pub fn new(projects_dir: PathBuf) -> Self {
        ClaudeCode {
            projects_dir,
            files: BTreeMap::new(),
            projects: HashMap::new(),
            vocabs: HashMap::new(),
            vocab_failed: HashSet::new(),
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
        for (rel, path) in &files {
            let (off_key, last_key, open_key) = (format!("off:{rel}"), format!("last_ts:{rel}"), format!("open:{rel}"));
            let off: u64 = state.get(&off_key).and_then(|v| v.parse().ok()).unwrap_or(0);
            let last_ts: Option<i64> = state.get(&last_key).and_then(|v| v.parse().ok());
            let chunk = match tail::read_lines(path, off, POLL_BUDGET) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("source claude-code: {}: {e}", path.display());
                    continue;
                }
            };
            if chunk.rotated {
                self.files.insert(rel.clone(), File { floor: last_ts, ..File::new(path) });
            } else if !self.files.contains_key(rel) {
                self.files.insert(rel.clone(), File::new(path));
                if off > 0 {
                    self.warm_up(rel, path, off, state.get(&open_key));
                }
            }
            let mut out = Vec::new();
            skipped += self.feed(rel, chunk.lines.iter().map(Vec::as_slice), &mut out);
            let f = self.files.get_mut(rel).expect("registered");
            f.session.expire(now_ms, &f.sid, f.project.clone(), &mut out);
            f.session.agent_writes.clear(); // plan 08 consumes these
            if let Some(floor) = f.floor {
                out.retain(|e| e.ts > floor);
            }
            if let Some(m) = out.iter().map(|e| e.ts).chain(last_ts).max() {
                state.set(&last_key, &m.to_string());
            }
            state.set(&off_key, &chunk.end.to_string());
            let open = f.session.open_turn.as_ref().map_or("-".to_owned(), |(u, t)| format!("{u} {t}"));
            state.set(&open_key, &open);
            events.extend(out);
        }
        if skipped > 0 {
            eprintln!("source claude-code: skipped {skipped} unreadable lines");
        }
        let w = self.files.values().map(|f| f.session.limit()).fold(now_ms - WRITE_SLACK_MS, i64::min);
        Ok(Poll { events, watermark: Some(w) })
    }
}
