//! One read transaction over the store (WAL gives snapshot isolation), for the dashboard and `status`.
//! Everything here only reads.
use crate::store::{Store, StoreError};
use rusqlite::types::Value;
use rusqlite::{params_from_iter, Connection};
use tally_scoring::{Event, LedgerEntry};

/// A deferred read transaction on a connection; ends on drop.
pub(crate) struct ReadTx<'a>(&'a Connection);

impl<'a> ReadTx<'a> {
    pub(crate) fn begin(conn: &'a Connection) -> Result<Self, StoreError> {
        conn.execute_batch("BEGIN")?;
        Ok(ReadTx(conn))
    }
}
impl Drop for ReadTx<'_> {
    fn drop(&mut self) {
        let _ = self.0.execute_batch("COMMIT");
    }
}

pub struct Snapshot<'a> {
    store: &'a Store,
    _tx: ReadTx<'a>,
}

impl Store {
    /// Starts a read transaction; the first read pins the snapshot. Dropping it ends the transaction.
    pub fn snapshot(&mut self) -> Result<Snapshot<'_>, StoreError> {
        let store: &Store = self;
        Ok(Snapshot { store, _tx: ReadTx::begin(&store.conn)? })
    }
}

/// The SQL of `Snapshot::events_in` / `project_events_in` (also what the index test explains).
pub(crate) fn events_sql(kinds: usize, projects: usize) -> String {
    let marks = |from: usize, n: usize| (from..from + n).map(|i| format!("?{i}")).collect::<Vec<_>>().join(",");
    let proj = if projects == 0 { String::new() } else { format!(" AND project IN ({})", marks(6 + kinds, projects)) };
    format!(
        "SELECT json FROM events WHERE kind IN ({}) AND ts >= ?1 AND ts < ?2 AND (ts, id) > (?3, ?4){proj} ORDER BY ts, id LIMIT ?5",
        marks(6, kinds)
    )
}

impl Snapshot<'_> {
    pub fn horizon(&self) -> Result<Option<i64>, StoreError> {
        self.store.horizon()
    }
    pub fn finalized_through(&self) -> Result<i64, StoreError> {
        self.store.finalized_through()
    }
    pub fn project_names(&self) -> Result<std::collections::BTreeMap<String, String>, StoreError> {
        self.store.project_names()
    }
    pub fn watermarks(&self) -> Result<Vec<(String, i64)>, StoreError> {
        let mut st = self.store.conn.prepare("SELECT source, ts FROM watermarks ORDER BY source")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
    /// Local project ids from the `projects` table.
    pub fn project_ids(&self) -> Result<Vec<String>, StoreError> {
        let mut st = self.store.conn.prepare("SELECT local_id FROM projects ORDER BY rowid")?;
        let rows = st.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
    /// Distinct projects seen in the ledger (a public name or a local id).
    pub fn ledger_projects(&self) -> Result<Vec<String>, StoreError> {
        let mut st = self.store.conn.prepare("SELECT DISTINCT json_extract(entry_json, '$.project') FROM ledger WHERE json_extract(entry_json, '$.project') IS NOT NULL ORDER BY 1")?;
        let rows = st.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
    /// Cheap counts for the dashboard: `(events, final_claims, verified_s, unverified_s, skipped_claims, provisional)`.
    /// Never loads event rows. `provisional` counts human-action events after `finalized_through`
    /// (ponytail: an estimate of what `status` reports by scoring everything; exact only if every action scores).
    pub fn counts(&self) -> Result<(u64, u64, u64, u64, u64, u64), StoreError> {
        let c = &self.store.conn;
        let events: i64 = c.query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))?;
        let (n, v, u): (i64, i64, i64) = c.query_row(
            "SELECT COUNT(*), COALESCE(SUM(json_extract(entry_json,'$.verified_s')),0), COALESCE(SUM(json_extract(entry_json,'$.unverified_s')),0) FROM ledger",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        let skipped: i64 = c.query_row("SELECT COUNT(*) FROM skipped_claims", [], |r| r.get(0))?;
        let fin = self.finalized_through()?;
        let prov: i64 = c.query_row(
            "SELECT COUNT(*) FROM events WHERE ts > ?1 AND (kind IN ('prompt','interrupt','tool_decision','user_edit')
               OR (kind = 'test_run' AND json_extract(json,'$.initiator') = 'user') OR (kind = 'commit' AND json_extract(json,'$.by') = 'user'))",
            [fin],
            |r| r.get(0),
        )?;
        Ok((events as u64, n as u64, v as u64, u as u64, skipped as u64, prov as u64))
    }
    pub fn ledger_len(&self) -> Result<u64, StoreError> {
        Ok(self.store.conn.query_row("SELECT COUNT(*) FROM ledger", [], |r| r.get::<_, i64>(0))? as u64)
    }
    /// Rows with `from_seq <= seq < to_seq_exclusive`, in order.
    pub fn ledger_range(&self, from_seq: u64, to_seq_exclusive: u64) -> Result<Vec<(u64, LedgerEntry)>, StoreError> {
        let mut st = self.store.conn.prepare("SELECT seq, entry_json FROM ledger WHERE seq >= ?1 AND seq < ?2 ORDER BY seq")?;
        let rows = st.query_map([from_seq as i64, to_seq_exclusive as i64], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
        let mut out = Vec::new();
        for r in rows {
            let (seq, j) = r?;
            out.push((seq as u64, serde_json::from_str(&j)?));
        }
        Ok(out)
    }
    /// Smallest `seq` (1-based) whose entry `ts >= ts`, or `ledger_len + 1`. Binary search by primary key:
    /// the ledger is a prefix of time-ordered claims.
    pub fn ledger_seq_at_or_after(&self, ts: i64) -> Result<u64, StoreError> {
        let (mut lo, mut hi) = (1u64, self.ledger_len()? + 1);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let at: i64 = self.store.conn.query_row("SELECT json_extract(entry_json, '$.ts') FROM ledger WHERE seq = ?1", [mid as i64], |r| r.get(0))?;
            if at >= ts {
                hi = mid;
            } else {
                lo = mid + 1;
            }
        }
        Ok(lo)
    }
    /// Skipped claims `(action_id, ts, reason)` with `from_ts <= ts < to_ts_exclusive`, in ts order.
    pub fn skipped_in(&self, from_ts: i64, to_ts_exclusive: i64) -> Result<Vec<(String, i64, String)>, StoreError> {
        let mut st = self.store.conn.prepare(
            "SELECT action_id, ts, reason FROM skipped_claims WHERE ts >= ?1 AND ts < ?2 ORDER BY ts, action_id",
        )?;
        let rows = st.query_map([from_ts, to_ts_exclusive], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
    /// The later of the last ledger entry and the last skipped claim, both strictly before `ts`.
    pub fn last_human_ts_before(&self, ts: i64) -> Result<Option<i64>, StoreError> {
        let seq = self.ledger_seq_at_or_after(ts)?;
        let ledger: Option<i64> = if seq > 1 {
            Some(self.store.conn.query_row("SELECT json_extract(entry_json, '$.ts') FROM ledger WHERE seq = ?1", [(seq - 1) as i64], |r| r.get(0))?)
        } else {
            None
        };
        let skipped: Option<i64> = self.store.conn.query_row("SELECT MAX(ts) FROM skipped_claims WHERE ts < ?1", [ts], |r| r.get(0))?;
        Ok(ledger.into_iter().chain(skipped).max())
    }

    pub fn events_in(&self, kinds: &[&str], from_ts: i64, to_ts: i64, after: Option<(i64, String)>, limit: usize) -> Result<Vec<Event>, StoreError> {
        self.project_events_in(&[], kinds, from_ts, to_ts, after, limit)
    }
    /// Events with `ts` in `[from_ts, to_ts)` of the given kinds (and projects when non-empty), ordered by
    /// `(ts, id)`, strictly after the cursor, at most `limit`. All values are bound parameters.
    pub fn project_events_in(&self, projects: &[String], kinds: &[&str], from_ts: i64, to_ts: i64, after: Option<(i64, String)>, limit: usize) -> Result<Vec<Event>, StoreError> {
        let (cts, cid) = after.unwrap_or((i64::MIN, String::new()));
        let mut args: Vec<Value> = vec![from_ts.into(), to_ts.into(), cts.into(), cid.into(), (limit as i64).into()];
        args.extend(kinds.iter().map(|k| Value::from(k.to_string())));
        args.extend(projects.iter().map(|p| Value::from(p.clone())));
        let mut st = self.store.conn.prepare(&events_sql(kinds.len(), projects.len()))?;
        let rows = st.query_map(params_from_iter(args), |r| r.get::<_, String>(0))?;
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
    use crate::test_support::{ev, prompt};
    use tally_scoring::{Actor, Kind};

    fn run(id: &str, ts: i64) -> Event {
        ev(id, ts, Kind::TestRun { cmd_hash: "h".into(), exit_code: 0, initiator: Actor::User })
    }
    fn entry(seq: i64, ts: i64) -> LedgerEntry {
        LedgerEntry { v: 1, ruleset: "r".into(), action_id: format!("a{seq}"), ts, project: None, kind: "prompt".into(), verified_s: 1, unverified_s: 0, reasons: vec![], features: Default::default(), prev_hash: String::new(), hash: format!("h{seq}") }
    }
    fn store_with_ledger(ts: &[i64]) -> Store {
        let s = Store::open_in_memory().unwrap();
        for (i, t) in ts.iter().enumerate() {
            let e = entry(i as i64 + 1, *t);
            s.conn.execute("INSERT INTO ledger(hash, entry_json) VALUES (?1, ?2)", rusqlite::params![e.hash, serde_json::to_string(&e).unwrap()]).unwrap();
        }
        s
    }

    #[test]
    fn migration_adds_generated_columns_once_and_old_rows_get_values() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("t.db");
        {
            let c = Connection::open(&path).unwrap();
            c.execute_batch("CREATE TABLE events(id TEXT PRIMARY KEY, ts INTEGER NOT NULL, json TEXT NOT NULL);").unwrap();
            c.execute("INSERT INTO events VALUES ('a', 1, ?1)", [serde_json::to_string(&prompt("a", 1, 500)).unwrap()]).unwrap();
        }
        let s = Store::open(&path).unwrap();
        let got: (String, Option<String>) = s.conn.query_row("SELECT kind, project FROM events", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!(got, ("prompt".into(), Some("p1".into())));
        drop(s);
        Store::open(&path).unwrap();
    }

    #[test]
    fn kind_window_query_uses_the_kind_ts_index() {
        let s = Store::open_in_memory().unwrap();
        let mut st = s.conn.prepare(&format!("EXPLAIN QUERY PLAN {}", events_sql(1, 0))).unwrap();
        let plan: Vec<String> = st.query_map(rusqlite::params![0, 1, i64::MIN, "", 5, "test_run"], |r| r.get::<_, String>(3)).unwrap().map(Result::unwrap).collect();
        let plan = plan.join(" | ");
        assert!(plan.contains("events_kind_ts"), "{plan}");
        assert!(!plan.contains("SCAN events"), "{plan}");
    }

    #[test]
    fn events_in_filters_pages_and_never_repeats() {
        let mut s = Store::open_in_memory().unwrap();
        let evs: Vec<Event> = (1..=7).map(|i| if i % 2 == 0 { prompt(&format!("e{i}"), i * 10, 1) } else { run(&format!("e{i}"), i * 10) }).collect();
        s.append_events(&evs).unwrap();
        let snap = s.snapshot().unwrap();
        let p1 = snap.events_in(&["test_run"], 0, 100, None, 2).unwrap();
        assert_eq!(p1.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["e1", "e3"]);
        let last = (p1[1].ts, p1[1].id.clone());
        let p2 = snap.events_in(&["test_run"], 0, 100, Some(last), 2).unwrap();
        assert_eq!(p2.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["e5", "e7"]);
        // from inclusive, to exclusive
        let w = snap.events_in(&["test_run"], 30, 70, None, 10).unwrap();
        assert_eq!(w.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["e3", "e5"]);
        assert_eq!(snap.project_events_in(&["nope".into()], &["test_run"], 0, 100, None, 10).unwrap().len(), 0);
        assert_eq!(snap.project_events_in(&["p1".into()], &["test_run", "prompt"], 0, 100, None, 10).unwrap().len(), 7);
    }

    #[test]
    fn ledger_seq_at_or_after_binary_search() {
        let mut s = store_with_ledger(&[10, 20, 20, 30]);
        let snap = s.snapshot().unwrap();
        let got: Vec<u64> = [5, 20, 21, 31].iter().map(|t| snap.ledger_seq_at_or_after(*t).unwrap()).collect();
        assert_eq!(got, [1, 2, 4, 5]);
        assert_eq!(snap.ledger_range(2, 4).unwrap().len(), 2);
        drop(snap);
        assert_eq!(Store::open_in_memory().unwrap().snapshot().unwrap().ledger_seq_at_or_after(5).unwrap(), 1);
    }

    #[test]
    fn last_human_ts_takes_the_later_of_ledger_and_skipped() {
        let mut s = store_with_ledger(&[10, 20]);
        s.conn.execute("INSERT INTO skipped_claims VALUES ('x', 25, 'r')", []).unwrap();
        let snap = s.snapshot().unwrap();
        assert_eq!(snap.last_human_ts_before(26).unwrap(), Some(25));
        assert_eq!(snap.last_human_ts_before(25).unwrap(), Some(20));
        assert_eq!(snap.last_human_ts_before(10).unwrap(), None);
        assert_eq!(snap.skipped_in(0, 25).unwrap().len(), 0);
        assert_eq!(snap.skipped_in(25, 26).unwrap().len(), 1);
    }

    #[test]
    fn read_only_handle_cannot_write_but_sees_new_commits() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("t.db");
        let mut a = Store::open(&path).unwrap();
        let mut r = Store::open_read_only(&path).unwrap();
        assert!(r.conn.execute("INSERT INTO meta VALUES ('k', 1)", []).is_err());
        a.append_events(&[run("e1", 10)]).unwrap();
        assert_eq!(r.snapshot().unwrap().events_in(&["test_run"], 0, 100, None, 10).unwrap().len(), 1);
        assert!(Store::open_read_only(&d.path().join("missing.db")).is_err());
    }
}
