//! Event builders shared by the daemon's unit tests.
use tally_scoring::{Event, Kind};

pub fn ev(id: &str, ts: i64, kind: Kind) -> Event {
    Event { v: 1, id: id.into(), ts, source: "test".into(), project: Some("p1".into()), session: None, kind }
}
pub fn prompt(id: &str, ts: i64, rel: u16) -> Event {
    ev(id, ts, Kind::Prompt { chars: 200, relevance_pm: rel, novelty_bytes: 100 })
}
pub fn edit(id: &str, ts: i64) -> Event {
    ev(id, ts, Kind::UserEdit { file_hash: "f".into(), lines_changed: 1 })
}
