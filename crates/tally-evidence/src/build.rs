//! Small constructors for synthetic events (fixed ids, no randomness).
use tally_scoring::{Actor, Decision, Event, Kind};

pub fn ev(id: impl Into<String>, ts: i64, kind: Kind) -> Event {
    Event { v: 1, id: id.into(), ts, source: "evidence".into(), project: Some("p".into()), session: Some("s".into()), kind }
}
pub fn prompt(id: &str, ts: i64, chars: u32, relevance_pm: u16, novelty_bytes: u32) -> Event {
    ev(id, ts, Kind::Prompt { chars, relevance_pm, novelty_bytes })
}
pub fn busy(id: &str, ts: i64, end_ts: i64) -> Event {
    ev(id, ts, Kind::AgentBusy { end_ts })
}
pub fn idle(id: &str, ts: i64, end_ts: i64) -> Event {
    ev(id, ts, Kind::Idle { end_ts })
}
pub fn edit(id: &str, ts: i64, lines: u32) -> Event {
    ev(id, ts, Kind::UserEdit { file_hash: format!("f{id}"), lines_changed: lines })
}
pub fn interrupt(id: &str, ts: i64) -> Event {
    ev(id, ts, Kind::Interrupt)
}
pub fn approve(id: &str, ts: i64) -> Event {
    ev(id, ts, Kind::ToolDecision { decision: Decision::Approve })
}
pub fn agent_output(id: &str, ts: i64, chars: u32) -> Event {
    ev(id, ts, Kind::AgentOutput { chars })
}
pub fn test_run(id: &str, ts: i64, cmd_hash: &str, exit_code: i32, initiator: Actor) -> Event {
    ev(id, ts, Kind::TestRun { cmd_hash: cmd_hash.into(), exit_code, initiator })
}
pub fn commit(id: &str, ts: i64, by: Actor, lines_changed: u32) -> Event {
    ev(id, ts, Kind::Commit { sha: format!("sha{id}"), by, lines_changed })
}
