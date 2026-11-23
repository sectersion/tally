//! WakaTime-compatible heartbeat protocol types, auth and file hash.
//! Wire format is UNVERIFIED, see tests/fixtures/heartbeat/NOTES.md.
pub mod server;

use crate::source::{Health, Poll, Source, SourceError, SourceState};
use base64::Engine;
use serde::Deserialize;
use server::{Received, Running, ServerConfig};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tally_scoring::{Event, Kind};

/// A heartbeat within this long before-or-at an agent write to the same file is the editor reacting to it.
pub const AGENT_WRITE_WINDOW_MS: i64 = 10_000;
/// Heartbeats wait this long in memory so agent writes seen later in logs can still match.
pub const HEARTBEAT_HOLD_MS: i64 = 15_000;
pub const HEARTBEAT_SLACK_MS: i64 = 60_000;
pub const EDIT_BUCKET_MS: i64 = 120_000;
/// How far back `sources()` seeds the agent-write list from the store.
pub const SEED_WINDOW_MS: i64 = 10 * 60_000;

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Heartbeat {
    pub entity: String,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    /// Float seconds since epoch.
    pub time: f64,
    pub category: Option<String>,
    pub project: Option<String>,
    pub branch: Option<String>,
    pub language: Option<String>,
    pub is_write: Option<bool>,
    pub lines: Option<u32>,
    pub lineno: Option<u32>,
    pub cursorpos: Option<u32>,
    pub line_additions: Option<u32>,
    pub line_deletions: Option<u32>,
    pub ai_line_changes: Option<u32>,
    pub human_line_changes: Option<u32>,
    pub user_agent: Option<String>,
    pub plugin: Option<String>,
}

impl Heartbeat {
    pub fn time_ms(&self) -> i64 {
        (self.time * 1000.0).round() as i64
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BodyError {
    #[error("malformed json")]
    Json,
    #[error("body is neither object nor array")]
    Shape,
}

/// A bulk array yields one result per element; an object yields one.
pub fn parse_body(bytes: &[u8]) -> Result<Vec<Result<Heartbeat, String>>, BodyError> {
    let v: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| BodyError::Json)?;
    let one = |v: serde_json::Value| serde_json::from_value::<Heartbeat>(v).map_err(|e| e.to_string());
    match v {
        serde_json::Value::Array(a) => Ok(a.into_iter().map(one).collect()),
        o @ serde_json::Value::Object(_) => Ok(vec![one(o)]),
        _ => Err(BodyError::Shape),
    }
}

/// Compares over the longer length with no early return.
pub(crate) fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    let n = a.len().max(b.len());
    let mut d = (a.len() ^ b.len()) as u64;
    for i in 0..n {
        d |= (a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0)) as u64;
    }
    d == 0
}

/// Accepts `Basic base64(key)`, `Bearer key`, or `?api_key=key`.
pub fn check_auth(header: Option<&str>, query_key: Option<&str>, token: &str) -> bool {
    let mut ok = false;
    if let Some(h) = header {
        let h = h.trim();
        if let Some(b) = h.strip_prefix("Basic ") {
            if let Ok(k) = base64::engine::general_purpose::STANDARD.decode(b.trim()) {
                ok |= ct_eq(&k, token.as_bytes());
            }
        } else if let Some(k) = h.strip_prefix("Bearer ") {
            ok |= ct_eq(k.trim().as_bytes(), token.as_bytes());
        }
    }
    if let Some(k) = query_key {
        ok |= ct_eq(k.as_bytes(), token.as_bytes());
    }
    ok
}

/// Never stores or returns the path. Hash of "<project_id>\0<path relative to the git root, '/' separators>".
pub fn file_hash(project_id: &str, rel_path: &str) -> String {
    let mut h = Sha256::new();
    h.update(project_id.as_bytes());
    h.update([0]);
    h.update(rel_path.as_bytes());
    hex::encode(h.finalize())
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Stats {
    pub accepted: usize,
    pub dropped_agent: usize,
    pub outside_project: usize,
    pub late_dropped: usize,
    pub not_a_file: usize,
}

pub struct Mapped {
    pub events: Vec<Event>,
    pub stats: Stats,
}

/// Path of `entity` under `root` (both normalized, component-aware, case-insensitive on Windows and macOS).
pub(crate) fn rel_under(root: &str, entity: &str) -> Option<String> {
    let fold = cfg!(any(windows, target_os = "macos"));
    let (r, e) = if fold { (root.to_lowercase(), entity.to_lowercase()) } else { (root.to_string(), entity.to_string()) };
    let rest = e.strip_prefix(r.trim_end_matches('/'))?.strip_prefix('/')?;
    // Take the tail from the original so its case survives (lowercasing can change byte length only for exotic chars).
    let rel = entity.get(entity.len().checked_sub(rest.len())?..)?;
    (!rel.is_empty()).then(|| rel.to_string())
}

/// Turns one batch of heartbeats into `user_edit` events. The path is hashed and dropped here.
pub fn map_heartbeats(
    batch: &[Received],
    project_of: &dyn Fn(&Path) -> Option<(PathBuf, String)>,
    agent_writes: &dyn Fn(&str, i64, i64) -> bool,
    floor: i64,
    prev_lines: &mut BTreeMap<String, u32>,
) -> Mapped {
    let mut hbs: Vec<&Heartbeat> = batch.iter().map(|r| &r.hb).collect();
    hbs.sort_by(|a, b| a.time.total_cmp(&b.time));
    let (mut events, mut stats, mut seen) = (Vec::new(), Stats::default(), BTreeSet::new());
    for hb in hbs {
        if hb.kind.as_deref().is_some_and(|k| k != "file") {
            stats.not_a_file += 1;
            continue;
        }
        let ent = crate::project::normalize_path(&hb.entity);
        let dir = ent.rsplit_once('/').map_or("", |x| x.0);
        let Some((root, pid)) = project_of(Path::new(dir)) else {
            stats.outside_project += 1;
            continue;
        };
        let Some(rel) = rel_under(&crate::project::normalize_path(&root.to_string_lossy()), &ent) else {
            stats.outside_project += 1;
            continue;
        };
        let h = file_hash(&pid, &rel);
        // Reading also sends heartbeats: an edit is an explicit write or a change in line count.
        let prev = hb.lines.and_then(|l| prev_lines.insert(h.clone(), l));
        let delta = hb.lines.zip(prev).map(|(l, p)| l.abs_diff(p)).filter(|d| *d > 0);
        if hb.is_write != Some(true) && delta.is_none() {
            stats.not_a_file += 1;
            continue;
        }
        let ts = hb.time_ms();
        if agent_writes(&h, ts - AGENT_WRITE_WINDOW_MS, ts) {
            stats.dropped_agent += 1;
            continue;
        }
        if ts <= floor {
            stats.late_dropped += 1;
            continue;
        }
        let bucket = ts.div_euclid(EDIT_BUCKET_MS);
        if !seen.insert((h.clone(), bucket)) {
            continue;
        }
        stats.accepted += 1;
        events.push(Event {
            v: 1,
            id: format!("heartbeat:{h}:{bucket}:user_edit"),
            ts,
            source: "heartbeat".into(),
            project: Some(pid),
            session: None,
            kind: Kind::UserEdit { file_hash: h, lines_changed: delta.unwrap_or(1) },
        });
    }
    Mapped { events, stats }
}

pub struct HeartbeatSource {
    sink: Arc<Mutex<Vec<Received>>>,
    running: Option<Running>,
    agent: Vec<(String, i64)>,
    prev_lines: BTreeMap<String, u32>,
    projects: HashMap<PathBuf, (PathBuf, String)>,
    totals: Stats,
    last_event_ts: Option<i64>,
    health: Option<Health>,
}

impl HeartbeatSource {
    /// No server: heartbeats come from `sink`. `seed` is the recent agent writes from the store.
    pub fn new(sink: Arc<Mutex<Vec<Received>>>, seed: Vec<(String, i64)>) -> Self {
        Self { sink, running: None, agent: seed, prev_lines: BTreeMap::new(), projects: HashMap::new(), totals: Stats::default(), last_event_ts: None, health: None }
    }

    pub fn start_with(cfg: ServerConfig, seed: Vec<(String, i64)>) -> std::io::Result<Self> {
        let sink = Arc::new(Mutex::new(Vec::new()));
        let mut s = Self::new(sink.clone(), seed);
        s.running = Some(server::start(cfg, sink)?);
        Ok(s)
    }

    pub fn start(port: u16, token_file: &Path, seed: Vec<(String, i64)>) -> std::io::Result<Self> {
        Self::start_with(ServerConfig::new(port, server::load_or_create_token(token_file)?), seed)
    }

    pub fn addr(&self) -> Option<std::net::SocketAddr> {
        self.running.as_ref().map(|r| r.addr)
    }
}

impl Source for HeartbeatSource {
    fn id(&self) -> &str {
        "heartbeat"
    }

    fn poll(&mut self, now_ms: i64, _state: &mut dyn SourceState) -> Result<Poll, SourceError> {
        let floor = now_ms - HEARTBEAT_HOLD_MS - HEARTBEAT_SLACK_MS;
        let batch: Vec<Received> = {
            let mut g = self.sink.lock().unwrap_or_else(|e| e.into_inner());
            let (ready, wait) = g.drain(..).partition(|r: &Received| r.hb.time_ms() <= now_ms - HEARTBEAT_HOLD_MS);
            *g = wait;
            ready
        };
        self.agent.retain(|(_, t)| *t >= now_ms - SEED_WINDOW_MS);
        let agent = &self.agent;
        let projects = std::cell::RefCell::new(&mut self.projects);
        let project_of = |d: &Path| {
            if let Some(p) = projects.borrow().get(d) {
                return Some(p.clone());
            }
            let p = crate::project::project_for_dir(d)?;
            projects.borrow_mut().insert(d.to_path_buf(), p.clone());
            Some(p)
        };
        let m = map_heartbeats(&batch, &project_of, &|h, from, to| agent.iter().any(|(ah, t)| ah == h && *t >= from && *t <= to), floor, &mut self.prev_lines);
        let t = &mut self.totals;
        t.accepted += m.stats.accepted;
        t.dropped_agent += m.stats.dropped_agent;
        t.outside_project += m.stats.outside_project;
        t.late_dropped += m.stats.late_dropped;
        t.not_a_file += m.stats.not_a_file;
        self.last_event_ts = m.events.iter().map(|e| e.ts).max().or(self.last_event_ts);
        let c = |k: &str, v: usize| (k.to_string(), v as u64);
        self.health = Some(Health {
            files_found: 0,
            records: t.accepted,
            skipped: t.dropped_agent + t.outside_project + t.late_dropped + t.not_a_file,
            errors: 0,
            format: "wakatime".into(),
            last_event_ts: self.last_event_ts,
            counters: vec![c("accepted", t.accepted), c("dropped_agent", t.dropped_agent), c("outside_project", t.outside_project), c("late_dropped", t.late_dropped), c("not_a_file", t.not_a_file)],
        });
        Ok(Poll { events: m.events, watermark: Some(floor) })
    }

    fn take_health(&mut self) -> Option<Health> {
        self.health.take()
    }

    fn see_agent_writes(&mut self, writes: &[(String, i64)]) {
        self.agent.extend_from_slice(writes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const SINGLE: &str = include_str!("../../tests/fixtures/heartbeat/single.json");
    const BULK: &str = include_str!("../../tests/fixtures/heartbeat/bulk.json");

    #[test]
    fn parses_single_and_bulk_fixtures() {
        let s = parse_body(SINGLE.as_bytes()).unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].as_ref().unwrap().kind.as_deref(), Some("file"));
        assert_eq!(parse_body(BULK.as_bytes()).unwrap().len(), 3);
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let r = parse_body(SINGLE.as_bytes()).unwrap();
        assert!(r[0].is_ok()); // fixture carries future_field
    }

    #[test]
    fn missing_entity_or_time_is_an_error_for_that_heartbeat_only() {
        let r = parse_body(BULK.as_bytes()).unwrap();
        assert!(r[0].is_ok() && r[1].is_err() && r[2].is_ok());
        let r = parse_body(br#"[{"entity":"a"},{"entity":"a","time":1}]"#).unwrap();
        assert!(r[0].is_err() && r[1].is_ok());
        assert!(parse_body(b"{nope").is_err());
        assert!(parse_body(b"3").is_err());
    }

    #[test]
    fn time_float_seconds_to_ms() {
        let r = parse_body(SINGLE.as_bytes()).unwrap();
        assert_eq!(r[0].as_ref().unwrap().time_ms(), 1763800000123);
    }

    #[test]
    fn auth_accepts_basic_base64_key_and_query_key_and_bearer() {
        let b64 = base64::engine::general_purpose::STANDARD.encode("k");
        assert!(check_auth(Some(&format!("Basic {b64}")), None, "k"));
        assert!(check_auth(None, Some("k"), "k"));
        assert!(check_auth(Some("Bearer k"), None, "k"));
    }

    #[test]
    fn auth_rejects_wrong_key_with_401_and_constant_time_compare() {
        let b64 = base64::engine::general_purpose::STANDARD.encode("kk");
        assert!(!check_auth(Some(&format!("Basic {b64}")), None, "k"));
        assert!(!check_auth(Some("Bearer x"), Some("y"), "k"));
        assert!(!check_auth(None, None, "k"));
        assert!(!check_auth(Some("Basic !!!"), None, "k"));
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abcd") && !ct_eq(b"abc", b"abd") && !ct_eq(b"", b"\0"));
    }

    #[test]
    fn file_hash_is_stable_and_path_free() {
        let a = file_hash("p1", "src/main.rs");
        assert_eq!(a, file_hash("p1", "src/main.rs"));
        assert_ne!(a, file_hash("p2", "src/main.rs"));
        assert_ne!(a, file_hash("p1", "src/lib.rs"));
        assert_eq!(a.len(), 64);
        assert!(!a.contains("main"));
    }

    // ---- map_heartbeats ----
    const T: i64 = 1_763_800_000_000;
    const ROOT: &str = "/repo";

    fn hb(path: &str, ms: i64, write: bool, lines: Option<u32>) -> Received {
        let h = Heartbeat {
            entity: path.into(),
            kind: Some("file".into()),
            time: ms as f64 / 1000.0,
            category: None,
            project: None,
            branch: None,
            language: None,
            is_write: Some(write),
            lines,
            lineno: None,
            cursorpos: None,
            line_additions: None,
            line_deletions: None,
            ai_line_changes: None,
            human_line_changes: None,
            user_agent: None,
            plugin: None,
        };
        Received { hb: h, received_ms: ms }
    }

    fn stub(dir: &Path) -> Option<(PathBuf, String)> {
        let d = crate::project::normalize_path(&dir.to_string_lossy()).to_lowercase();
        (d.starts_with("/repo") || d.starts_with("c:/repo")).then(|| (PathBuf::from(if d.starts_with("c:") { "c:/repo" } else { ROOT }), "p".into()))
    }

    fn run(batch: &[Received], agent: &[(String, i64)], floor: i64) -> Mapped {
        map_heartbeats(batch, &stub, &|h, a, b| agent.iter().any(|(x, t)| x == h && *t >= a && *t <= b), floor, &mut BTreeMap::new())
    }

    #[test]
    fn outside_project_is_counted_and_has_no_event() {
        let m = run(&[hb("/elsewhere/secret-name/a.rs", T, true, None)], &[], 0);
        assert_eq!((m.events.len(), m.stats.outside_project), (0, 1));
        let dump = format!("{:?}{}", m.stats, serde_json::to_string(&m.events).unwrap());
        assert!(!dump.contains("secret-name") && !dump.contains("elsewhere"));
    }

    #[test]
    fn agent_write_within_window_discards() {
        let h = file_hash("p", "src/a.rs");
        let w = [(h, T)];
        let p = "/repo/src/a.rs";
        assert_eq!(run(&[hb(p, T + 9_999, true, None)], &w, 0).stats.dropped_agent, 1);
        let m = run(&[hb(p, T + 10_001, true, None)], &w, 0);
        assert_eq!((m.stats.dropped_agent, m.events.len()), (0, 1));
    }

    #[test]
    fn agent_write_for_another_file_does_not_discard() {
        let w = [(file_hash("p", "src/b.rs"), T)];
        assert_eq!(run(&[hb("/repo/src/a.rs", T + 1_000, true, None)], &w, 0).events.len(), 1);
    }

    #[test]
    fn heartbeat_before_the_agent_write_is_kept() {
        let w = [(file_hash("p", "src/a.rs"), T + 5_000)];
        assert_eq!(run(&[hb("/repo/src/a.rs", T, true, None)], &w, 0).events.len(), 1);
    }

    #[test]
    fn out_of_order_bulk_gives_same_events_as_sorted() {
        let sorted: Vec<Received> = (0..10).map(|i| hb(&format!("/repo/src/f{}.rs", i % 4), T + i * 70_000, true, None)).collect();
        let mut shuffled = sorted.clone();
        for (i, j) in [(0, 9), (1, 7), (2, 5), (3, 8), (4, 6)] {
            shuffled.swap(i, j);
        }
        let a = run(&sorted, &[], 0);
        let b = run(&shuffled, &[], 0);
        assert_eq!(a.events, b.events);
        assert_eq!(a.stats, b.stats);
    }

    #[test]
    fn bulk_of_25_with_one_late_heartbeat_keeps_the_rest() {
        let floor = T;
        let mut batch: Vec<Received> = (1..25).map(|i| hb(&format!("/repo/src/f{i}.rs"), T + i * 1_000, true, None)).collect();
        batch.push(hb("/repo/src/late.rs", floor, true, None));
        let m = run(&batch, &[], floor);
        assert_eq!((m.stats.accepted, m.stats.late_dropped, m.events.len()), (24, 1, 24));
    }

    #[test]
    fn heartbeats_in_one_bucket_make_one_event_and_two_buckets_two() {
        let b0 = T.div_euclid(EDIT_BUCKET_MS) * EDIT_BUCKET_MS;
        let p = "/repo/src/a.rs";
        let m = run(&[hb(p, b0 + 50_000, true, None), hb(p, b0 + 10_000, true, None), hb(p, b0 + 100_000, true, None)], &[], 0);
        assert_eq!(m.events.len(), 1);
        assert_eq!(m.events[0].ts, b0 + 10_000); // earliest decides
        let m = run(&[hb(p, b0 + 10_000, true, None), hb(p, b0 + EDIT_BUCKET_MS + 10_000, true, None)], &[], 0);
        assert_eq!(m.events.len(), 2);
    }

    #[test]
    fn read_only_cursor_movement_is_not_an_edit() {
        let p = "/repo/src/a.rs";
        let m = run(&[hb(p, T, false, Some(10)), hb(p, T + 1_000, false, Some(10))], &[], 0);
        assert_eq!((m.events.len(), m.stats.not_a_file), (0, 2));
        let mut app = hb(p, T, true, None);
        app.hb.kind = Some("app".into());
        assert_eq!(run(&[app], &[], 0).stats.not_a_file, 1);
    }

    #[test]
    fn lines_changed_is_the_delta_or_one() {
        let p = "/repo/src/a.rs";
        let b0 = T.div_euclid(EDIT_BUCKET_MS) * EDIT_BUCKET_MS;
        // not a write, but lines went 10 -> 17: an edit of 7 lines
        let m = run(&[hb(p, b0, false, Some(10)), hb(p, b0 + EDIT_BUCKET_MS, false, Some(17))], &[], 0);
        assert_eq!(m.events.len(), 1);
        assert!(matches!(m.events[0].kind, Kind::UserEdit { lines_changed: 7, .. }));
        let m = run(&[hb(p, b0, true, None)], &[], 0);
        assert!(matches!(m.events[0].kind, Kind::UserEdit { lines_changed: 1, .. }));
    }

    #[test]
    fn nested_repo_maps_to_the_inner_root() {
        let d = tempfile::tempdir().unwrap();
        let outer = d.path().join("outer");
        let inner = outer.join("inner");
        std::fs::create_dir_all(outer.join(".git")).unwrap();
        std::fs::create_dir_all(inner.join(".git")).unwrap();
        std::fs::create_dir_all(inner.join("src")).unwrap();
        let file = inner.join("src").join("a.rs");
        let m = map_heartbeats(&[hb(&file.to_string_lossy(), T, true, None)], &crate::project::project_for_dir, &|_, _, _| false, 0, &mut BTreeMap::new());
        let (_, inner_id) = crate::project::project_for_dir(&inner).unwrap();
        assert_eq!(m.events[0].project.as_deref(), Some(inner_id.as_str()));
        assert!(matches!(&m.events[0].kind, Kind::UserEdit { file_hash: h, .. } if *h == file_hash(&inner_id, "src/a.rs")));
    }

    #[cfg(windows)]
    #[test]
    fn path_with_backslashes_and_drive_case_matches_on_windows() {
        let m = run(&[hb(r"C:\Repo\src\a.rs", T, true, None)], &[], 0);
        assert_eq!(m.events.len(), 1);
        assert!(matches!(&m.events[0].kind, Kind::UserEdit { file_hash: h, .. } if *h == file_hash("p", "src/a.rs")));
    }

    #[test]
    fn events_roundtrip_through_store_and_score() {
        use crate::store::Store;
        let mut s = Store::open_in_memory().unwrap();
        let m = run(&[hb("/repo/src/a.rs", T + 60_000, true, None)], &[], 0);
        let mut evs = m.events;
        evs.push(crate::test_support::prompt("pr", T, 600));
        s.append_events(&evs).unwrap();
        s.set_watermark("heartbeat", T + 1_000_000).unwrap();
        crate::finalize::finalize(&mut s).unwrap();
        assert!(crate::status::status(&s).unwrap().verified_s > 0);
    }

    #[test]
    fn poll_holds_recent_heartbeats_and_returns_floor_watermark() {
        let sink = Arc::new(Mutex::new(vec![hb("/repo/src/a.rs", T, true, None)]));
        let mut src = HeartbeatSource::new(sink.clone(), vec![]);
        let mut st = BTreeMap::new();
        let p = src.poll(T + 10_000, &mut st).unwrap(); // inside the hold
        assert!(p.events.is_empty());
        assert_eq!(sink.lock().unwrap().len(), 1);
        let p = src.poll(T + HEARTBEAT_HOLD_MS, &mut st).unwrap();
        assert_eq!(p.watermark, Some(T - HEARTBEAT_SLACK_MS));
        assert!(sink.lock().unwrap().is_empty());
        assert!(src.take_health().is_some());
    }
}
