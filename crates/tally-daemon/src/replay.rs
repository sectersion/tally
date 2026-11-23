//! Session list and replay timeline, built from event kinds, times and numbers only (no text exists to show).
//! Sessions are a display notion (gap of at most `REPLAY_SESSION_GAP_MS` between claims of one project), not a
//! ruleset constant. Skipped claims neither extend nor split a session.
use crate::analytics::{find_bug_fixes, load_events, HUMAN_AND_RUN_KINDS};
use crate::snapshot::Snapshot;
use crate::store::StoreError;
use serde::Serialize;
use std::collections::BTreeMap;
use tally_scoring::ruleset::LOOP_WINDOW_MS;
use tally_scoring::{Actor, Decision, Event, Kind, LedgerEntry};

pub const REPLAY_SESSION_GAP_MS: i64 = 30 * 60 * 1000;
pub const REPLAY_PAGE_MAX: usize = 2000;
const SESSIONS_MAX: usize = 500;
const REPLAY_KINDS: [&str; 8] = ["prompt", "interrupt", "tool_decision", "user_edit", "test_run", "commit", "agent_busy", "idle"];

#[derive(Serialize, Debug, PartialEq)]
pub struct Session {
    pub project: Option<String>,
    pub start_ts: i64,
    pub end_ts: i64,
    pub actions: u32,
    pub verified_s: u64,
    pub unverified_s: u64,
}

/// Newest first.
pub fn sessions(entries: &[(u64, LedgerEntry)]) -> Vec<Session> {
    let mut open: BTreeMap<Option<String>, Session> = BTreeMap::new();
    let mut done = Vec::new();
    for (_, e) in entries {
        let cur = open.get_mut(&e.project);
        match cur {
            Some(s) if e.ts - s.end_ts <= REPLAY_SESSION_GAP_MS => {
                s.end_ts = e.ts;
                s.actions += 1;
                s.verified_s += u64::from(e.verified_s);
                s.unverified_s += u64::from(e.unverified_s);
            }
            _ => {
                let fresh = Session { project: e.project.clone(), start_ts: e.ts, end_ts: e.ts, actions: 1, verified_s: u64::from(e.verified_s), unverified_s: u64::from(e.unverified_s) };
                if let Some(old) = open.insert(e.project.clone(), fresh) {
                    done.push(old);
                }
            }
        }
    }
    done.extend(open.into_values());
    done.sort_by(|a, b| (b.end_ts, &b.project).cmp(&(a.end_ts, &a.project)));
    done
}

pub fn sessions_for(snap: &Snapshot, from_ts: i64, to_ts: i64, limit: usize) -> Result<Vec<Session>, StoreError> {
    let (a, b) = (snap.ledger_seq_at_or_after(from_ts)?, snap.ledger_seq_at_or_after(to_ts)?);
    let mut s = sessions(&snap.ledger_range(a, b)?);
    s.truncate(limit.clamp(1, SESSIONS_MAX));
    Ok(s)
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Item {
    pub ts: i64,
    pub end_ts: Option<i64>,
    pub kind: &'static str,
    pub n: Option<i64>,
    pub short: Option<String>,
    pub agent: bool,
    pub project: Option<String>,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Page {
    pub items: Vec<Item>,
    /// `"<ts>:<id>"` of the last item when more remain.
    pub next: Option<String>,
    pub truncated: bool,
}

fn short(h: &str) -> Option<String> {
    Some(h.chars().take(8).collect())
}

fn item(e: &Event, fix: Option<&crate::analytics::BugFix>) -> Option<Item> {
    let it = |kind, n: Option<i64>, short: Option<String>, agent| Item { ts: e.ts, end_ts: None, kind, n, short, agent, project: e.project.clone() };
    Some(match &e.kind {
        Kind::Prompt { relevance_pm, .. } => it("prompt", Some(i64::from(*relevance_pm)), None, false),
        Kind::Interrupt => it("interrupt", None, None, false),
        Kind::ToolDecision { decision: Decision::Approve } => it("approval", None, None, false),
        Kind::ToolDecision { decision: Decision::Deny } => it("denial", None, None, false),
        Kind::UserEdit { lines_changed, .. } => it("edit", Some(i64::from(*lines_changed)), None, false),
        Kind::TestRun { cmd_hash, exit_code, initiator } => {
            let kind = match fix {
                _ if *exit_code != 0 => "error",
                Some(f) if f.by_agent => "agent_fix",
                Some(f) if !f.skipped => "fix",
                _ => "run",
            };
            it(kind, Some(i64::from(*exit_code)), short(cmd_hash), *initiator == Actor::Agent)
        }
        Kind::Commit { sha, by, lines_changed } => it("commit", Some(i64::from(*lines_changed)), short(sha), *by == Actor::Agent),
        Kind::AgentBusy { end_ts } => Item { end_ts: Some(*end_ts), ..it("agent_working", None, None, true) },
        Kind::Idle { end_ts } => Item { end_ts: Some(*end_ts), ..it("away", None, None, false) },
        Kind::AgentOutput { .. } => return None,
    })
}

/// `projects` are local project ids (empty means all). A passing run that completed a bug fix is `fix`
/// (`agent_fix` when the agent ran it); a fix whose claim was skipped is drawn as `run`.
pub fn replay_page(snap: &Snapshot, projects: &[String], from_ts: i64, to_ts: i64, after: Option<(i64, String)>, limit: usize) -> Result<Page, StoreError> {
    let limit = limit.clamp(1, REPLAY_PAGE_MAX);
    let evs = snap.project_events_in(projects, &REPLAY_KINDS, from_ts, to_ts, after.clone(), limit + 1)?;
    let skipped = snap.skipped_in(from_ts, to_ts)?;
    let fix_end = evs.last().map_or(to_ts, |e| e.ts.saturating_add(1)).min(to_ts);
    let loop_events = load_events(snap, &HUMAN_AND_RUN_KINDS, from_ts.saturating_sub(LOOP_WINDOW_MS), fix_end)?;
    let mut fixes: BTreeMap<String, crate::analytics::BugFix> = find_bug_fixes(&loop_events, from_ts, fix_end).into_iter().map(|f| (f.id.clone(), f)).collect();
    for s in &skipped {
        if let Some(f) = fixes.get_mut(&s.0).filter(|f| !f.by_agent) {
            f.skipped = true;
        }
    }
    let mut all: Vec<(i64, String, Item)> = evs.iter().filter_map(|e| item(e, fixes.get(&e.id)).map(|i| (e.ts, e.id.clone(), i))).collect();
    if projects.is_empty() {
        for (id, ts, _) in &skipped {
            if after.as_ref().is_none_or(|(ats, aid)| (*ts, id) > (*ats, aid)) {
                all.push((*ts, id.clone(), Item { ts: *ts, end_ts: None, kind: "skipped", n: None, short: None, agent: false, project: None }));
            }
        }
    }
    all.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
    let more = all.len() > limit;
    all.truncate(limit);
    let next = if more { all.last().map(|(ts, id, _)| format!("{ts}:{id}")) } else { None };
    Ok(Page { truncated: next.is_some(), next, items: all.into_iter().map(|x| x.2).collect() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    use crate::test_support::{ev, prompt};

    fn entry(ts_min: i64, project: &str) -> (u64, LedgerEntry) {
        (1, LedgerEntry { v: 1, ruleset: "r".into(), action_id: "a".into(), ts: ts_min * 60_000, project: Some(project.into()), kind: "prompt".into(), verified_s: 10, unverified_s: 1, reasons: vec![], features: Default::default(), prev_hash: String::new(), hash: String::new() })
    }
    fn run(id: &str, s: i64, exit: i32, by: Actor) -> Event {
        ev(id, s * 1000, Kind::TestRun { cmd_hash: "SECRETCMDHASH0123".into(), exit_code: exit, initiator: by })
    }
    fn store(evs: &[Event], skipped: &[&str]) -> Store {
        let mut s = Store::open_in_memory().unwrap();
        s.append_events(evs).unwrap();
        for id in skipped {
            let ts = evs.iter().find(|e| e.id == *id).unwrap().ts;
            s.conn.execute("INSERT INTO skipped_claims VALUES (?1, ?2, 'no_presence_coverage')", rusqlite::params![id, ts]).unwrap();
        }
        s
    }
    fn page(s: &mut Store, projects: &[String], limit: usize) -> Page {
        let snap = s.snapshot().unwrap();
        replay_page(&snap, projects, 0, 1_000_000_000, None, limit).unwrap()
    }
    fn kinds(p: &Page) -> Vec<&'static str> {
        p.items.iter().map(|i| i.kind).collect()
    }

    #[test]
    fn sessions_split_on_a_gap_over_thirty_minutes_per_project() {
        let rows = [entry(0, "p1"), entry(5, "p2"), entry(10, "p1"), entry(40, "p1"), entry(71, "p1")];
        let s = sessions(&rows);
        assert_eq!(s.len(), 3);
        assert_eq!((s[0].start_ts, s[0].actions), (71 * 60_000, 1));
        let p1_first = s.iter().find(|x| x.project.as_deref() == Some("p1") && x.start_ts == 0).unwrap();
        assert_eq!((p1_first.actions, p1_first.end_ts, p1_first.verified_s, p1_first.unverified_s), (3, 40 * 60_000, 30, 3));
    }
    #[test]
    fn replay_maps_kinds_and_numbers_without_text() {
        let evs = [prompt("a", 1000, 640), run("b", 10, 1, Actor::User), prompt("c", 20_000, 500), run("d", 30, 0, Actor::User),
            ev("e", 40_000, Kind::Commit { sha: "abcdef0123456789".into(), by: Actor::User, lines_changed: 12 })];
        let mut s = store(&evs, &[]);
        let p = page(&mut s, &[], 100);
        assert_eq!(kinds(&p), ["prompt", "error", "prompt", "fix", "commit"]);
        assert_eq!(p.items.iter().map(|i| i.n).collect::<Vec<_>>(), [Some(640), Some(1), Some(500), Some(0), Some(12)]);
        let json = serde_json::to_string(&p).unwrap();
        for k in ["cmd_hash", "file_hash", "\"sha\"", "text", "SECRETCMDHASH0123", "abcdef0123456789"] {
            assert!(!json.contains(k), "{k}");
        }
        assert!(json.contains("SECRETCM") && json.contains("abcdef01"));
    }
    #[test]
    fn an_agent_fix_is_drawn_as_agent_fix_and_a_skipped_fix_as_run() {
        let evs = [run("f", 0, 1, Actor::User), prompt("p", 10_000, 500), run("claude:s:u:test_run", 20, 0, Actor::Agent),
            run("f2", 100, 1, Actor::User), prompt("p2", 110_000, 500), run("ok2", 120, 0, Actor::User)];
        let mut s = store(&evs, &["ok2"]);
        let p = page(&mut s, &[], 100);
        assert_eq!(kinds(&p), ["error", "prompt", "agent_fix", "error", "prompt", "run", "skipped"]);
        assert!(p.items[2].agent);
        // with a project filter there is no skipped row
        let p = page(&mut s, &["p1".into()], 100);
        assert!(!kinds(&p).contains(&"skipped") && !kinds(&p).contains(&"fix"));
    }
    #[test]
    fn spans_become_agent_working_and_away_bars() {
        let evs = [ev("b", 10_000, Kind::AgentBusy { end_ts: 20_000 }), ev("i", 30_000, Kind::Idle { end_ts: 50_000 }), ev("o", 31_000, Kind::AgentOutput { chars: 5 })];
        let mut s = store(&evs, &[]);
        let p = page(&mut s, &[], 100);
        assert_eq!(kinds(&p), ["agent_working", "away"]);
        assert_eq!((p.items[0].end_ts, p.items[1].end_ts), (Some(20_000), Some(50_000)));
    }
    #[test]
    fn pages_are_complete_and_disjoint() {
        let evs: Vec<Event> = (0..25).map(|i| prompt(&format!("e{i:02}"), i * 1000, 100)).collect();
        let mut s = store(&evs, &[]);
        let snap = s.snapshot().unwrap();
        let (mut got, mut after) = (Vec::new(), None);
        let mut sizes = Vec::new();
        loop {
            let p = replay_page(&snap, &[], 0, 1_000_000, after.clone(), 10).unwrap();
            sizes.push(p.items.len());
            got.extend(p.items.iter().map(|i| i.ts));
            match p.next {
                Some(n) => {
                    let (ts, id) = n.split_once(':').unwrap();
                    after = Some((ts.parse().unwrap(), id.to_string()));
                }
                None => break,
            }
        }
        assert_eq!(sizes, [10, 10, 5]);
        assert_eq!(got, (0..25).map(|i| i * 1000).collect::<Vec<_>>());
    }
    #[test]
    fn project_filter_returns_only_that_projects_events() {
        let mut other = prompt("o", 2000, 100);
        other.project = Some("p2".into());
        let mut s = store(&[prompt("a", 1000, 100), other], &[]);
        assert_eq!(page(&mut s, &["p2".into()], 10).items.len(), 1);
        assert_eq!(page(&mut s, &[], 10).items.len(), 2);
    }
    #[test]
    fn limit_is_clamped_and_truncation_is_reported() {
        let evs: Vec<Event> = (0..3).map(|i| prompt(&format!("e{i}"), i * 1000, 100)).collect();
        let mut s = store(&evs, &[]);
        let p = page(&mut s, &[], 0);
        assert_eq!((p.items.len(), p.truncated), (1, true));
        assert_eq!(page(&mut s, &[], 10_000).items.len(), 3);
    }
}
