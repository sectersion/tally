//! GitHub Copilot (Copilot CLI / agent host) adapter. Reads `<copilot home>/session-state/<id>/events.jsonl`.
//! Format notes and evidence: `tests/fixtures/copilot/NOTES.md`. Every record is a loose `serde_json::Value`
//! (unknown fields never fail); text is reduced to numbers and dropped.
//!
//! Supported: prompt, agent_output, agent_busy. Not recorded in any log seen so far (so never emitted):
//! interrupt, tool_decision (every observed approval was `host_policy`, not the user), agent writes.
use super::common::{file_watermark, iso_to_ms, stable_id};
use super::{Adapter, ParseCtx, ParseReport, Parsed, RealCtx};
use crate::claude_code::features::ContextTail;
use crate::claude_code::session::OPEN_SPAN_MAX_MS;
use crate::project::normalize_path;
use crate::source::{Health, Poll, Source, SourceError, SourceState};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tally_scoring::{Event, Kind};

const SOURCE: &str = "copilot";
/// Live capture only: session files untouched for longer are ignored.
const MAX_AGE_MS: i64 = 7 * 24 * 3_600_000;

pub struct Copilot;

impl Adapter for Copilot {
    const ID: &'static str = SOURCE;
    fn parse(text: &str, ctx: &dyn ParseCtx) -> (Parsed, ParseReport) {
        let (p, r, _) = parse_full(text, ctx);
        (p, r)
    }
}

fn chars(s: &str) -> u32 {
    u32::try_from(s.chars().count()).unwrap_or(u32::MAX)
}

fn ev(session: &str, project: &Option<String>, native: &str, kind_s: &str, ts: i64, kind: Kind) -> Event {
    Event {
        v: 1,
        id: stable_id(SOURCE, &format!("{session}:{native}"), kind_s),
        ts,
        source: SOURCE.into(),
        project: project.clone(),
        session: Some(session.to_string()),
        kind,
    }
}

fn emit(out: &mut Parsed, rep: &mut ParseReport, e: Event) {
    rep.last_ts = rep.last_ts.max(Some(e.ts));
    out.events.push(e);
}

/// Like `Adapter::parse`, plus the start of the turn still open at the end of the text (it holds the watermark).
pub fn parse_full(text: &str, ctx: &dyn ParseCtx) -> (Parsed, ParseReport, Option<i64>) {
    let mut out = Parsed { events: vec![], agent_write_hashes: vec![] };
    let mut rep = ParseReport { format: "unknown".into(), ..Default::default() };
    let (mut session, mut project, mut root): (Option<String>, Option<String>, Option<PathBuf>) = (None, None, None);
    let (mut tail, mut last_output, mut last_error) = (ContextTail::default(), String::new(), String::new());
    // (native id, start ts, ts of the latest assistant.turn_end since)
    let mut open: Option<(String, i64, Option<i64>)> = None;
    macro_rules! close {
        ($force_end:expr) => {
            if let (Some((nid, start, end)), Some(s)) = (open.take(), session.as_deref()) {
                if let Some(end) = $force_end.or(end) {
                    emit(&mut out, &mut rep, ev(s, &project, &nid, "agent_busy", start, Kind::AgentBusy { end_ts: end.max(start) }));
                }
            }
        };
    }
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        rep.records += 1;
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            rep.errors += 1;
            continue;
        };
        let Some(ts) = v.get("timestamp").and_then(Value::as_str).and_then(iso_to_ms) else {
            rep.errors += 1;
            continue;
        };
        let (Some(ty), data) = (v.get("type").and_then(Value::as_str), v.get("data")) else {
            rep.skipped += 1;
            continue;
        };
        let str_of = |k: &str| data.and_then(|d| d.get(k)).and_then(Value::as_str);
        if ty == "session.start" {
            session = str_of("sessionId").map(str::to_string);
            if session.is_some() {
                rep.format = match data.and_then(|d| d.get("version")).and_then(Value::as_u64) {
                    Some(n) => format!("cli-events-v{n}"),
                    None => "unknown".into(),
                };
            }
            let cwd = data.and_then(|d| d.get("context")).and_then(|c| c.get("cwd")).and_then(Value::as_str);
            (root, project) = match cwd.and_then(|c| ctx.project_for_dir(c)) {
                Some((r, p)) => (Some(r), Some(p)),
                None => (None, None),
            };
            rep.skipped += 1;
            continue;
        }
        let Some(sess) = session.clone() else {
            rep.skipped += 1; // nothing can be attributed before (or without) a session.start
            continue;
        };
        match ty {
            "user.message" => {
                // Only `delivery == "idle"` is seen in real logs; any other value is not guessed to be human input.
                let content = str_of("content").filter(|c| !c.is_empty());
                match (str_of("delivery"), content) {
                    (Some("idle"), Some(text)) => {
                        close!(None::<i64>);
                        let (c, rel, nov) = ctx.prompt_features(root.as_deref(), text, &last_output, &last_error, &tail);
                        let nid = str_of("messageId").map_or_else(|| format!("i{i}"), str::to_string);
                        emit(&mut out, &mut rep, ev(&sess, &project, &nid, "prompt", ts, Kind::Prompt { chars: c, relevance_pm: rel, novelty_bytes: nov }));
                        tail.push(text);
                        open = Some((nid, ts, None));
                    }
                    _ => rep.skipped += 1,
                }
            }
            "assistant.message" => match str_of("content").filter(|c| !c.is_empty()) {
                Some(text) => {
                    let nid = str_of("messageId").map_or_else(|| format!("i{i}"), str::to_string);
                    emit(&mut out, &mut rep, ev(&sess, &project, &nid, "agent_output", ts, Kind::AgentOutput { chars: chars(text) }));
                    tail.push(text);
                    last_output = text.to_string();
                }
                None => rep.skipped += 1,
            },
            "assistant.turn_end" => {
                if let Some(o) = open.as_mut() {
                    o.2 = Some(ts);
                }
                rep.skipped += 1;
            }
            "tool.execution_complete" => {
                if data.and_then(|d| d.get("success")).and_then(Value::as_bool) == Some(false) {
                    last_error = data.and_then(|d| d.get("result")).and_then(|r| r.get("content")).and_then(Value::as_str).unwrap_or("").to_string();
                }
                rep.skipped += 1;
            }
            "hook.start" if str_of("hookType") == Some("agentStop") => {
                close!(None::<i64>);
                rep.skipped += 1;
            }
            "session.shutdown" => {
                close!(None::<i64>);
                rep.skipped += 1;
            }
            _ => rep.skipped += 1,
        }
    }
    (out, rep, open.map(|o| o.1))
}

/// Copilot CLI home: `COPILOT_HOME` (docs, UNVERIFIED) else `~/.copilot`.
pub fn default_home() -> Option<PathBuf> {
    std::env::var_os("COPILOT_HOME").map(PathBuf::from).or_else(|| dirs::home_dir().map(|h| h.join(".copilot")))
}

struct Seen {
    stat: String,
    open: Option<i64>,
    report: ParseReport,
}

pub struct CopilotSource {
    home: Option<PathBuf>,
    ctx: RealCtx,
    /// Per file: the stat it was parsed at and what that parse gave. Memory only: after a restart every file is
    /// parsed once more and id dedup absorbs the repeats.
    // ponytail: whole-file re-parse on growth (files are append-only but a few MB at most); switch to byte cursors if sessions reach tens of MB.
    seen: BTreeMap<PathBuf, Seen>,
    health: Option<Health>,
}

impl CopilotSource {
    pub fn new(home: Option<PathBuf>) -> Self {
        CopilotSource { home, ctx: RealCtx::default(), seen: BTreeMap::new(), health: None }
    }
}

fn session_files(home: &Path, now_ms: i64) -> Vec<(PathBuf, String)> {
    let Ok(rd) = std::fs::read_dir(home.join("session-state")) else { return vec![] };
    let mut v: Vec<_> = rd
        .filter_map(Result::ok)
        .map(|e| e.path().join("events.jsonl"))
        .filter_map(|p| {
            let m = std::fs::metadata(&p).ok()?;
            let mtime = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as i64;
            (now_ms - mtime <= MAX_AGE_MS).then(|| (p, format!("{}:{mtime}", m.len())))
        })
        .collect();
    v.sort();
    v
}

impl Source for CopilotSource {
    fn id(&self) -> &str {
        SOURCE
    }

    fn poll(&mut self, now_ms: i64, _state: &mut dyn SourceState) -> Result<Poll, SourceError> {
        let files = self.home.as_deref().map(|h| session_files(h, now_ms)).unwrap_or_default();
        self.seen.retain(|p, _| files.iter().any(|(f, _)| f == p));
        let mut events = vec![];
        for (path, stat) in &files {
            if self.seen.get(path).is_some_and(|s| &s.stat == stat) {
                continue;
            }
            let text = match std::fs::read(path) {
                Ok(b) => String::from_utf8_lossy(&b).into_owned(),
                Err(e) => {
                    // Name only, never content. The file keeps its old parse (and its hold on the watermark),
                    // counts as one error so doctor does not say ok, and is retried next poll (empty stat).
                    eprintln!("source copilot: {}: {}", normalize_path(&path.to_string_lossy()), e.kind());
                    let s = self.seen.entry(path.clone()).or_insert_with(|| Seen { stat: String::new(), open: None, report: ParseReport::default() });
                    s.stat.clear();
                    s.report.records += 1;
                    s.report.errors += 1;
                    continue;
                }
            };
            let (parsed, report, open) = parse_full(&text, &self.ctx);
            events.extend(parsed.events);
            self.seen.insert(path.clone(), Seen { stat: stat.clone(), open, report });
        }
        let mut h = Health { files_found: files.len(), format: String::new(), ..Default::default() };
        for s in self.seen.values() {
            h.records += s.report.records;
            h.skipped += s.report.skipped;
            h.errors += s.report.errors;
            h.last_event_ts = h.last_event_ts.max(s.report.last_ts);
            if h.format.is_empty() || h.format == "unknown" {
                h.format = s.report.format.clone();
            }
        }
        self.health = Some(h);
        // An abandoned open turn (killed session) stops holding the watermark after the span limit.
        let opens = self.seen.values().filter_map(|s| s.open).filter(|o| now_ms - o <= OPEN_SPAN_MAX_MS);
        Ok(Poll { events, watermark: Some(file_watermark(now_ms, opens)) })
    }

    fn take_health(&mut self) -> Option<Health> {
        self.health.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_kit::StubCtx;
    use std::collections::BTreeMap;

    fn iso(ms: i64) -> String {
        chrono::DateTime::from_timestamp_millis(ms).unwrap().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    }
    fn line(ty: &str, ms: i64, data: serde_json::Value) -> String {
        serde_json::json!({"type": ty, "data": data, "id": format!("i{ms}"), "timestamp": iso(ms)}).to_string() + "\n"
    }
    fn start(ms: i64, cwd: &str) -> String {
        line("session.start", ms, serde_json::json!({"sessionId": "s1", "version": 1, "context": {"cwd": cwd}}))
    }
    fn prompt(ms: i64, id: &str, text: &str, delivery: &str) -> String {
        line("user.message", ms, serde_json::json!({"content": text, "delivery": delivery, "messageId": id}))
    }
    fn stop(ms: i64) -> String {
        line("assistant.turn_end", ms - 1, serde_json::json!({"turnId": "0"})) + &line("hook.start", ms, serde_json::json!({"hookType": "agentStop"}))
    }
    fn kinds(p: &Parsed) -> Vec<&'static str> {
        p.events
            .iter()
            .map(|e| match e.kind {
                Kind::Prompt { .. } => "prompt",
                Kind::AgentBusy { .. } => "busy",
                Kind::AgentOutput { .. } => "output",
                _ => "other",
            })
            .collect()
    }

    #[test]
    fn prompt_busy_and_output_come_out() {
        let t = start(0, "C:/tallyproj")
            + &prompt(1_000, "m1", "hello", "idle")
            + &line("assistant.message", 2_000, serde_json::json!({"content": "four", "messageId": "a1"}))
            + &stop(3_000);
        let (p, r, open) = parse_full(&t, &StubCtx);
        assert_eq!(kinds(&p), ["prompt", "output", "busy"]);
        assert_eq!(p.events[0].id, "copilot:s1:m1:prompt");
        assert_eq!(p.events[0].project.as_deref(), Some("proj"));
        assert_eq!(p.events[1].kind, Kind::AgentOutput { chars: 4 });
        assert_eq!(p.events[2].kind, Kind::AgentBusy { end_ts: 2_999 });
        assert_eq!((r.format.as_str(), r.errors, open), ("cli-events-v1", 0, None));
    }

    #[test]
    fn agent_followups_and_unknown_deliveries_are_not_prompts() {
        let t = start(0, "C:/tallyproj") + &prompt(1_000, "m1", "queued text", "queued") + &prompt(2_000, "m2", "", "idle");
        let (p, r, _) = parse_full(&t, &StubCtx);
        assert!(p.events.is_empty() && r.skipped >= 3 && r.errors == 0);
    }

    #[test]
    fn open_turn_is_not_emitted_and_reports_its_start() {
        let t = start(0, "C:/tallyproj") + &prompt(1_000, "m1", "hello", "idle");
        let (p, _, open) = parse_full(&t, &StubCtx);
        assert_eq!((kinds(&p), open), (vec!["prompt"], Some(1_000)));
    }

    #[test]
    fn nothing_without_session_start_and_no_project_outside_a_repo() {
        let (p, r, _) = parse_full(&prompt(1_000, "m1", "hello", "idle"), &StubCtx);
        assert!(p.events.is_empty() && r.format == "unknown");
        let t = start(0, "D:/elsewhere") + &prompt(1_000, "m1", "hello", "idle");
        assert_eq!(parse_full(&t, &StubCtx).0.events[0].project, None);
    }

    #[test]
    fn bad_json_and_missing_timestamp_are_errors_not_panics() {
        let t = start(0, "C:/tallyproj") + "{\"type\":\"user.mes\n" + "{\"type\":\"x\",\"data\":{}}\n" + &prompt(1_000, "m1", "hi", "idle");
        let (p, r, _) = parse_full(&t, &StubCtx);
        assert_eq!((p.events.len(), r.errors), (1, 2));
    }

    fn home_with(text: &str) -> (tempfile::TempDir, PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("session-state").join("s1").join("events.jsonl");
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(&f, text).unwrap();
        (d, f)
    }
    fn now() -> i64 {
        crate::source::now_ms()
    }

    #[test]
    fn poll_emits_a_changed_file_once_and_appends_keep_ids() {
        let n = now();
        let t = start(n - 60_000, "C:/x") + &prompt(n - 50_000, "m1", "hello", "idle") + &stop(n - 40_000);
        let (d, f) = home_with(&t);
        let mut src = CopilotSource::new(Some(d.path().to_path_buf()));
        let mut st = BTreeMap::new();
        let first = src.poll(n, &mut st).unwrap().events;
        assert_eq!(first.len(), 2);
        assert!(src.poll(n, &mut st).unwrap().events.is_empty());
        std::fs::write(&f, t + &prompt(n - 30_000, "m2", "again", "idle")).unwrap();
        let again = src.poll(n, &mut st).unwrap().events;
        assert!(first.iter().all(|e| again.iter().any(|a| a.id == e.id)) && again.len() == 3);
    }

    #[test]
    fn open_turn_holds_the_watermark_and_closed_gives_now_minus_slack() {
        let n = now();
        let (d, f) = home_with(&(start(n - 210_000, "C:/x") + &prompt(n - 200_000, "m1", "hello", "idle")));
        let mut src = CopilotSource::new(Some(d.path().to_path_buf()));
        assert_eq!(src.poll(n, &mut BTreeMap::new()).unwrap().watermark, Some(n - 200_001));
        std::fs::write(&f, start(n - 210_000, "C:/x") + &prompt(n - 200_000, "m1", "hello", "idle") + &stop(n - 190_000)).unwrap();
        let w = src.poll(n, &mut BTreeMap::new()).unwrap().watermark;
        assert_eq!(w, Some(n - crate::claude_code::WRITE_SLACK_MS));
    }

    #[test]
    fn missing_home_is_no_logs_not_an_error() {
        let d = tempfile::tempdir().unwrap();
        for home in [Some(d.path().join("nope")), None] {
            let mut src = CopilotSource::new(home);
            let p = src.poll(now(), &mut BTreeMap::new()).unwrap();
            assert!(p.events.is_empty());
            assert_eq!(src.take_health().unwrap().files_found, 0);
            assert!(src.take_health().is_none());
        }
    }

    #[test]
    fn files_older_than_seven_days_are_ignored() {
        let n = now();
        let (d, _) = home_with(&(start(n, "C:/x") + &prompt(n, "m1", "hello", "idle")));
        let mut src = CopilotSource::new(Some(d.path().to_path_buf()));
        let p = src.poll(n + 8 * 24 * 3_600_000, &mut BTreeMap::new()).unwrap();
        assert!(p.events.is_empty());
        assert_eq!(src.take_health().unwrap().files_found, 0);
    }

    #[test]
    fn health_carries_format_and_counts() {
        let n = now();
        let (d, _) = home_with(&(start(n - 5_000, "C:/x") + &prompt(n - 4_000, "m1", "hello", "idle") + "garbage\n"));
        let mut src = CopilotSource::new(Some(d.path().to_path_buf()));
        src.poll(n, &mut BTreeMap::new()).unwrap();
        let h = src.take_health().unwrap();
        assert_eq!((h.files_found, h.records, h.errors, h.format.as_str(), h.last_event_ts), (1, 3, 1, "cli-events-v1", Some(n - 4_000)));
    }

    #[test]
    fn unreadable_events_file_counts_as_an_error() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("session-state/s1/events.jsonl")).unwrap(); // a directory: read fails
        let mut src = CopilotSource::new(Some(d.path().to_path_buf()));
        src.poll(now(), &mut BTreeMap::new()).unwrap();
        let h = src.take_health().unwrap();
        assert_eq!((h.files_found, h.records, h.errors), (1, 1, 1));
    }
}
