//! SQLite store. Keeps only what an `Event` holds (hashes and numbers); no event text.
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use std::time::Duration;
use std::collections::BTreeMap;
use std::path::Path;
use tally_scoring::{Event, LedgerEntry, GENESIS_HASH};

#[derive(thiserror::Error, Debug)]
pub enum StoreError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("late event {id} at ts {ts} is at or before finalized_through {finalized_through}")]
    LateEvent { id: String, ts: i64, finalized_through: i64 },
    #[error("ledger chain broken: first entry does not extend the stored chain")]
    ChainBroken,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppendSummary {
    pub added: usize,
    pub duplicates: usize,
}

pub struct Store {
    pub(crate) conn: Connection,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS events(id TEXT PRIMARY KEY, ts INTEGER NOT NULL, json TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS events_ts ON events(ts);
CREATE TABLE IF NOT EXISTS watermarks(source TEXT PRIMARY KEY, ts INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS projects(local_id TEXT PRIMARY KEY, root TEXT NOT NULL, public_name TEXT);
CREATE TABLE IF NOT EXISTS ledger(seq INTEGER PRIMARY KEY, hash TEXT NOT NULL, entry_json TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY, value INTEGER NOT NULL);
CREATE TRIGGER IF NOT EXISTS ledger_no_update BEFORE UPDATE ON ledger
BEGIN SELECT RAISE(ABORT, 'ledger is append-only'); END;
CREATE TRIGGER IF NOT EXISTS ledger_no_delete BEFORE DELETE ON ledger
BEGIN SELECT RAISE(ABORT, 'ledger is append-only'); END;
";

fn read_finalized(conn: &Connection) -> Result<i64, StoreError> {
    let v: Option<i64> = conn
        .query_row("SELECT value FROM meta WHERE key = 'finalized_through'", [], |r| r.get(0))
        .optional()?;
    Ok(v.unwrap_or(i64::MIN))
}

impl Store {
    pub fn open(path: &Path) -> Result<Store, StoreError> {
        let conn = Connection::open(path)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Store, StoreError> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Store, StoreError> {
        conn.execute_batch(SCHEMA)?;
        Ok(Store { conn })
    }

    /// One transaction. Already-stored ids are duplicates (checked first, even if late);
    /// otherwise `ts <= finalized_through` fails the whole batch with `LateEvent`.
    pub fn append_events(&mut self, events: &[Event]) -> Result<AppendSummary, StoreError> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let fin = read_finalized(&tx)?;
        let (mut added, mut duplicates) = (0, 0);
        for e in events {
            let seen: Option<i64> =
                tx.query_row("SELECT 1 FROM events WHERE id = ?1", [&e.id], |r| r.get(0)).optional()?;
            if seen.is_some() {
                duplicates += 1;
            } else if e.ts <= fin {
                return Err(StoreError::LateEvent { id: e.id.clone(), ts: e.ts, finalized_through: fin });
            } else {
                tx.execute(
                    "INSERT INTO events(id, ts, json) VALUES (?1, ?2, ?3)",
                    params![e.id, e.ts, serde_json::to_string(e)?],
                )?;
                added += 1;
            }
        }
        tx.commit()?;
        Ok(AppendSummary { added, duplicates })
    }

    /// All events sorted by `(ts, id)`.
    pub fn events(&self) -> Result<Vec<Event>, StoreError> {
        let mut st = self.conn.prepare("SELECT json FROM events ORDER BY ts, id")?;
        let rows = st.query_map([], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for j in rows {
            out.push(serde_json::from_str(&j?)?);
        }
        Ok(out)
    }

    /// Source contract: a source calls `set_watermark(source, ts)` to promise that it will
    /// never again emit an event with `ts <= watermark`, including span events (`idle`,
    /// `agent_busy`) whose start `ts` is at or below it. A source with an open span must
    /// therefore keep its watermark below the span's start until it emits the closed span.
    /// The stored value only moves forward.
    pub fn set_watermark(&mut self, source: &str, ts: i64) -> Result<(), StoreError> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO watermarks(source, ts) VALUES (?1, ?2)
             ON CONFLICT(source) DO UPDATE SET ts = MAX(ts, excluded.ts)",
            params![source, ts],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Minimum watermark over all sources; `None` if there are none.
    pub fn horizon(&self) -> Result<Option<i64>, StoreError> {
        Ok(self.conn.query_row("SELECT MIN(ts) FROM watermarks", [], |r| r.get(0))?)
    }

    /// `i64::MIN` until the first `append_ledger`.
    pub fn finalized_through(&self) -> Result<i64, StoreError> {
        read_finalized(&self.conn)
    }

    pub fn register_project(&mut self, local_id: &str, root: &str, public_name: Option<&str>) -> Result<(), StoreError> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO projects(local_id, root, public_name) VALUES (?1, ?2, ?3)
             ON CONFLICT(local_id) DO UPDATE SET root = excluded.root, public_name = excluded.public_name",
            params![local_id, root, public_name],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Local id to public name, only for projects that have a name.
    pub fn project_names(&self) -> Result<BTreeMap<String, String>, StoreError> {
        let mut st = self
            .conn
            .prepare("SELECT local_id, public_name FROM projects WHERE public_name IS NOT NULL")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// One transaction. `entries[0].prev_hash` must equal the last stored hash
    /// (`GENESIS_HASH` when empty). `finalized_through` only moves forward; an empty
    /// `entries` still moves it.
    pub fn append_ledger(&mut self, entries: &[LedgerEntry], finalized_through: i64) -> Result<(), StoreError> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let last: Option<String> = tx
            .query_row("SELECT hash FROM ledger ORDER BY seq DESC LIMIT 1", [], |r| r.get(0))
            .optional()?;
        if let Some(first) = entries.first() {
            if first.prev_hash != last.as_deref().unwrap_or(GENESIS_HASH) {
                return Err(StoreError::ChainBroken);
            }
        }
        for e in entries {
            tx.execute(
                "INSERT INTO ledger(hash, entry_json) VALUES (?1, ?2)",
                params![e.hash, serde_json::to_string(e)?],
            )?;
        }
        tx.execute(
            "INSERT INTO meta(key, value) VALUES ('finalized_through', ?1)
             ON CONFLICT(key) DO UPDATE SET value = MAX(value, excluded.value)",
            [finalized_through],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Ledger entries in append order.
    pub fn ledger(&self) -> Result<Vec<LedgerEntry>, StoreError> {
        let mut st = self.conn.prepare("SELECT entry_json FROM ledger ORDER BY seq")?;
        let rows = st.query_map([], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for j in rows {
            out.push(serde_json::from_str(&j?)?);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tally_scoring::event::Kind;
    use tally_scoring::{Event, LedgerEntry, GENESIS_HASH};

    fn ev(id: &str, ts: i64) -> Event {
        Event { v: 1, id: id.into(), ts, source: "test".into(), project: None, session: None, kind: Kind::Interrupt }
    }
    fn entry(action_id: &str, prev_hash: &str) -> LedgerEntry {
        let mut e = LedgerEntry {
            v: 1, ruleset: "t".into(), action_id: action_id.into(), ts: 1, project: None, kind: "k".into(),
            verified_s: 1, unverified_s: 0, reasons: vec![], features: Default::default(),
            prev_hash: prev_hash.into(), hash: String::new(),
        };
        e.hash = tally_scoring::ledger::entry_hash(&e);
        e
    }

    #[test] fn duplicates_are_ignored_and_events_come_back_sorted() {
        let mut s = Store::open_in_memory().unwrap();
        let r = s.append_events(&[ev("b", 20), ev("a", 10), ev("a", 10)]).unwrap();
        assert_eq!((r.added, r.duplicates), (2, 1));
        let ids: Vec<_> = s.events().unwrap().into_iter().map(|e| e.id).collect();
        assert_eq!(ids, ["a", "b"]);
    }
    #[test] fn late_events_reject_the_whole_batch_but_duplicates_pass() {
        let mut s = Store::open_in_memory().unwrap();
        s.append_events(&[ev("old", 500)]).unwrap();
        s.append_ledger(&[], 1000).unwrap();
        assert!(matches!(s.append_events(&[ev("ok", 1001), ev("late", 1000)]),
            Err(StoreError::LateEvent { ts: 1000, finalized_through: 1000, .. })));
        assert_eq!(s.events().unwrap().len(), 1);
        let r = s.append_events(&[ev("old", 500), ev("ok", 1001)]).unwrap();
        assert_eq!((r.added, r.duplicates), (1, 1));
    }
    #[test] fn horizon_is_the_minimum_watermark_and_watermarks_never_move_back() {
        let mut s = Store::open_in_memory().unwrap();
        assert_eq!(s.horizon().unwrap(), None);
        s.set_watermark("a", 100).unwrap(); s.set_watermark("b", 50).unwrap();
        assert_eq!(s.horizon().unwrap(), Some(50));
        s.set_watermark("b", 10).unwrap();
        assert_eq!(s.horizon().unwrap(), Some(50));
    }
    #[test] fn ledger_appends_in_order_and_rejects_a_broken_link() {
        let mut s = Store::open_in_memory().unwrap();
        let e1 = entry("a1", GENESIS_HASH);
        let e2 = entry("a2", &e1.hash);
        s.append_ledger(&[e1.clone(), e2.clone()], 10).unwrap();
        assert_eq!(s.ledger().unwrap(), vec![e1, e2]);
        assert!(matches!(s.append_ledger(&[entry("a3", GENESIS_HASH)], 20), Err(StoreError::ChainBroken)));
        assert_eq!(s.ledger().unwrap().len(), 2);
        assert_eq!(s.finalized_through().unwrap(), 10);
    }
    #[test] fn ledger_rows_cannot_be_updated_or_deleted() {
        let mut s = Store::open_in_memory().unwrap();
        s.append_ledger(&[entry("a1", GENESIS_HASH)], 10).unwrap();
        assert!(s.conn.execute("UPDATE ledger SET hash = 'x'", []).is_err());
        assert!(s.conn.execute("DELETE FROM ledger", []).is_err());
    }
    #[test] fn two_handles_on_one_file_see_each_others_writes() {
        let dir = tempfile::tempdir().unwrap(); let p = dir.path().join("tally.db");
        let mut a = Store::open(&p).unwrap();
        let mut b = Store::open(&p).unwrap();
        b.append_ledger(&[entry("a1", GENESIS_HASH)], 1000).unwrap();
        assert!(matches!(a.append_events(&[ev("late", 1000)]),
            Err(StoreError::LateEvent { ts: 1000, finalized_through: 1000, .. })));
        assert_eq!(a.ledger().unwrap().len(), 1);
    }
    #[test] fn data_survives_reopen_and_projects_are_named() {
        let dir = tempfile::tempdir().unwrap(); let p = dir.path().join("tally.db");
        { let mut s = Store::open(&p).unwrap();
          s.append_events(&[ev("a", 1)]).unwrap();
          s.register_project("id1", "C:/r", Some("owner/repo")).unwrap();
          s.register_project("id2", "C:/q", None).unwrap(); }
        let s = Store::open(&p).unwrap();
        assert_eq!(s.events().unwrap().len(), 1);
        assert_eq!(s.project_names().unwrap().get("id1").map(String::as_str), Some("owner/repo"));
        assert_eq!(s.project_names().unwrap().len(), 1);
    }
}
