//! SQLite store. Keeps only what an `Event` holds (hashes and numbers); no event text.
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use std::time::Duration;
use std::collections::BTreeMap;
use std::path::Path;
pub use crate::snapshot::Snapshot;
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
    #[error("events at or before the finalize horizon changed while finalizing; retry")]
    Conflict,
    #[error("project {0} is not registered; run tally project add first")]
    UnknownProject(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppendSummary {
    pub added: usize,
    pub duplicates: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRow {
    pub local_id: String,
    pub root: String,
    pub public_name: Option<String>,
    pub common_dir: Option<String>,
    pub remote: Option<String>,
}

/// A public-ledger row: the main-ledger `seq` it came from and its entry (`project` is the public name).
#[derive(Debug, Clone, PartialEq)]
pub struct PublishedRow {
    pub main_seq: i64,
    pub entry: LedgerEntry,
}

/// `(start, end)` in ms.
pub type Interval = (i64, i64);

pub struct Store {
    pub(crate) conn: Connection,
}

/// `source_state` owner of run-level facts (no source has this id).
const RUN_STATE: &str = "_run";
const DISABLED_KEY: &str = "disabled_sources";

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS events(id TEXT PRIMARY KEY, ts INTEGER NOT NULL, json TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS events_ts ON events(ts);
CREATE TABLE IF NOT EXISTS watermarks(source TEXT PRIMARY KEY, ts INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS projects(local_id TEXT PRIMARY KEY, root TEXT NOT NULL, public_name TEXT);
CREATE TABLE IF NOT EXISTS ledger(seq INTEGER PRIMARY KEY, hash TEXT NOT NULL, entry_json TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS source_state(source TEXT NOT NULL, key TEXT NOT NULL, value TEXT NOT NULL, PRIMARY KEY(source, key));
CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY, value INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS coverage(start_ts INTEGER NOT NULL, end_ts INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS adapter_health(source TEXT PRIMARY KEY, found_files INTEGER NOT NULL, records INTEGER NOT NULL, skipped INTEGER NOT NULL, errors INTEGER NOT NULL, format TEXT NOT NULL, last_event_ts INTEGER, counters TEXT NOT NULL, checked_ts INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS agent_writes(file_hash TEXT NOT NULL, ts INTEGER NOT NULL, PRIMARY KEY(file_hash, ts));
CREATE TABLE IF NOT EXISTS skipped_claims(action_id TEXT PRIMARY KEY, ts INTEGER NOT NULL, reason TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS skipped_ts ON skipped_claims(ts);
CREATE TRIGGER IF NOT EXISTS skipped_no_update BEFORE UPDATE ON skipped_claims
BEGIN SELECT RAISE(ABORT, 'skipped_claims is append-only'); END;
CREATE TRIGGER IF NOT EXISTS skipped_no_delete BEFORE DELETE ON skipped_claims
BEGIN SELECT RAISE(ABORT, 'skipped_claims is append-only'); END;
CREATE TABLE IF NOT EXISTS published_ledger(
  project TEXT NOT NULL, ruleset TEXT NOT NULL, n INTEGER NOT NULL,
  main_seq INTEGER NOT NULL, hash TEXT NOT NULL, entry_json TEXT NOT NULL,
  PRIMARY KEY(project, ruleset, n), UNIQUE(project, main_seq));
CREATE TRIGGER IF NOT EXISTS published_no_update BEFORE UPDATE ON published_ledger
BEGIN SELECT RAISE(ABORT, 'published ledger is append-only'); END;
CREATE TRIGGER IF NOT EXISTS published_no_delete BEFORE DELETE ON published_ledger
BEGIN SELECT RAISE(ABORT, 'published ledger is append-only'); END;
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
        conn.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()))?;
        conn.execute_batch("PRAGMA synchronous = NORMAL;")?;
        Self::init(conn)
    }

    /// Read-only handle for the dashboard: no schema, no migration, a missing file is an error.
    pub fn open_read_only(path: &Path) -> Result<Store, StoreError> {
        use rusqlite::OpenFlags;
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        Ok(Store { conn })
    }

    pub fn open_in_memory() -> Result<Store, StoreError> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Store, StoreError> {
        conn.execute_batch(SCHEMA)?;
        for col in ["common_dir", "remote", "server_id"] {
            let have: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_info('projects') WHERE name = ?1)",
                [col],
                |r| r.get(0),
            )?;
            if !have {
                // another process may have migrated between the check and here
                match conn.execute_batch(&format!("ALTER TABLE projects ADD COLUMN {col} TEXT")) {
                    Err(e) if e.to_string().contains("duplicate column name") => {}
                    r => r?,
                }
            }
        }
        // Virtual generated columns over the event JSON, so window queries by kind or project use an index.
        for (col, path) in [("kind", "$.kind"), ("project", "$.project")] {
            let have: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_xinfo('events') WHERE name = ?1)",
                [col],
                |r| r.get(0),
            )?;
            if !have {
                match conn.execute_batch(&format!(
                    "ALTER TABLE events ADD COLUMN {col} TEXT GENERATED ALWAYS AS (json_extract(json, '{path}')) VIRTUAL"
                )) {
                    Err(e) if e.to_string().contains("duplicate column name") => {}
                    r => r?,
                }
            }
        }
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS events_kind_ts ON events(kind, ts);
             CREATE INDEX IF NOT EXISTS events_project_ts ON events(project, ts);",
        )?;
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

    /// All state keys for one source.
    pub fn get_state(&self, source: &str) -> Result<BTreeMap<String, String>, StoreError> {
        let mut st = self.conn.prepare("SELECT key, value FROM source_state WHERE source = ?1")?;
        let rows = st.query_map([source], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// One transaction; upserts every pair, keys not in `values` are kept.
    pub fn set_state(&mut self, source: &str, values: &BTreeMap<String, String>) -> Result<(), StoreError> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        for (k, v) in values {
            tx.execute(
                "INSERT INTO source_state(source, key, value) VALUES (?1, ?2, ?3)
                 ON CONFLICT(source, key) DO UPDATE SET value = excluded.value",
                params![source, k, v],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Records agent file writes `(file_hash, ts)`; a re-emitted batch is harmless. Only the runner calls this.
    pub fn put_agent_writes(&mut self, w: &[(String, i64)]) -> Result<(), StoreError> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        for (h, ts) in w {
            tx.execute("INSERT OR IGNORE INTO agent_writes(file_hash, ts) VALUES (?1, ?2)", params![h, ts])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Agent writes with `ts >= since`, oldest first.
    pub fn agent_writes_since(&self, since: i64) -> Result<Vec<(String, i64)>, StoreError> {
        let mut st = self.conn.prepare("SELECT file_hash, ts FROM agent_writes WHERE ts >= ?1 ORDER BY ts, file_hash")?;
        let rows = st.query_map([since], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Upserts one source's adapter health. Only the runner calls this.
    pub fn put_health(&mut self, source: &str, h: &crate::source::Health, checked_ts: i64) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO adapter_health(source, found_files, records, skipped, errors, format, last_event_ts, counters, checked_ts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(source) DO UPDATE SET found_files = excluded.found_files, records = excluded.records,
               skipped = excluded.skipped, errors = excluded.errors, format = excluded.format,
               last_event_ts = excluded.last_event_ts, counters = excluded.counters, checked_ts = excluded.checked_ts",
            params![
                source,
                h.files_found as i64,
                h.records as i64,
                h.skipped as i64,
                h.errors as i64,
                h.format,
                h.last_event_ts,
                serde_json::to_string(&h.counters).unwrap_or_else(|_| "[]".into()),
                checked_ts
            ],
        )?;
        Ok(())
    }

    /// Every stored health row as `(source, health, checked_ts)`, ordered by source.
    pub fn get_health(&self) -> Result<Vec<(String, crate::source::Health, i64)>, StoreError> {
        let mut st = self.conn.prepare(
            "SELECT source, found_files, records, skipped, errors, format, last_event_ts, counters, checked_ts
             FROM adapter_health ORDER BY source",
        )?;
        let rows = st.query_map([], |r| {
            let counters: String = r.get(7)?;
            let h = crate::source::Health {
                files_found: r.get::<_, i64>(1)? as usize,
                records: r.get::<_, i64>(2)? as usize,
                skipped: r.get::<_, i64>(3)? as usize,
                errors: r.get::<_, i64>(4)? as usize,
                format: r.get(5)?,
                last_event_ts: r.get(6)?,
                counters: serde_json::from_str(&counters).unwrap_or_default(),
            };
            Ok((r.get(0)?, h, r.get(8)?))
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// One source's watermark; `None` if it never set one.
    pub fn watermark(&self, source: &str) -> Result<Option<i64>, StoreError> {
        Ok(self.conn.query_row("SELECT ts FROM watermarks WHERE source = ?1", [source], |r| r.get(0)).optional()?)
    }

    /// Records `presence_required` and the disabled list in one transaction, so a finalizer
    /// never sees one new and the other stale. See `set_disabled_sources` for the list.
    pub fn set_run_config(&mut self, presence_required: bool, disabled: &[&str]) -> Result<(), StoreError> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO meta(key, value) VALUES ('presence_required', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [i64::from(presence_required)],
        )?;
        tx.execute(
            "INSERT INTO source_state(source, key, value) VALUES (?1, ?2, ?3)
             ON CONFLICT(source, key) DO UPDATE SET value = excluded.value",
            params![RUN_STATE, DISABLED_KEY, disabled.join(",")],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Records the sources config.toml switches off; `horizon` skips only those rows. The list
    /// is as of the last `tally run` start. Re-enabling a source makes its stale watermark
    /// count again at the next start, and events between that watermark and `finalized_through`
    /// are then late (dropped).
    pub fn set_disabled_sources(&mut self, ids: &[&str]) -> Result<(), StoreError> {
        self.set_state(RUN_STATE, &BTreeMap::from([(DISABLED_KEY.to_string(), ids.join(","))]))
    }

    /// Minimum watermark over every source except those in the recorded disabled list (none
    /// skipped if `tally run` never recorded one, see `set_disabled_sources`); `None` if no
    /// rows count. A disabled source's old watermark row stays but no longer holds the horizon.
    pub fn horizon(&self) -> Result<Option<i64>, StoreError> {
        let disabled = self.get_state(RUN_STATE)?.remove(DISABLED_KEY).unwrap_or_default();
        let mut st = self.conn.prepare("SELECT source, ts FROM watermarks")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        let mut min: Option<i64> = None;
        for row in rows {
            let (source, ts) = row?;
            if !disabled.split(',').any(|i| i == source) {
                min = Some(min.map_or(ts, |m| m.min(ts)));
            }
        }
        Ok(min)
    }

    /// `i64::MIN` until the first `append_ledger`.
    pub fn finalized_through(&self) -> Result<i64, StoreError> {
        read_finalized(&self.conn)
    }

    pub fn register_project(&mut self, local_id: &str, root: &str, public_name: Option<&str>) -> Result<(), StoreError> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO projects(local_id, root, public_name) VALUES (?1, ?2, ?3)
             ON CONFLICT(local_id) DO UPDATE SET root = excluded.root,
             public_name = COALESCE(excluded.public_name, projects.public_name)",
            params![local_id, root, public_name],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Keeps a stored value when the new one is `None`.
    pub fn set_project_identity(&mut self, local_id: &str, id: &crate::project::RepoIdentity) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE projects SET common_dir = COALESCE(?2, common_dir), remote = COALESCE(?3, remote) WHERE local_id = ?1",
            params![local_id, id.common_dir, id.remote],
        )?;
        Ok(())
    }

    /// Sets the server's project id; the public name is untouched.
    pub fn set_server_id(&mut self, local_id: &str, server_id: &str) -> Result<(), StoreError> {
        let n = self.conn.execute("UPDATE projects SET server_id = ?2 WHERE local_id = ?1", params![local_id, server_id])?;
        if n == 0 {
            return Err(StoreError::UnknownProject(local_id.to_string()));
        }
        Ok(())
    }

    /// Public name to the distinct, sorted server ids of its local ids (only rows with both).
    pub fn server_ids(&self) -> Result<BTreeMap<String, Vec<String>>, StoreError> {
        let mut st = self.conn.prepare(
            "SELECT DISTINCT public_name, server_id FROM projects
             WHERE public_name IS NOT NULL AND server_id IS NOT NULL ORDER BY public_name, server_id",
        )?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for r in rows {
            let (name, sid) = r?;
            out.entry(name).or_default().push(sid);
        }
        Ok(out)
    }

    /// All projects in insertion order.
    pub fn projects(&self) -> Result<Vec<ProjectRow>, StoreError> {
        let mut st = self
            .conn
            .prepare("SELECT local_id, root, public_name, common_dir, remote FROM projects ORDER BY rowid")?;
        let rows = st.query_map([], |r| {
            Ok(ProjectRow { local_id: r.get(0)?, root: r.get(1)?, public_name: r.get(2)?, common_dir: r.get(3)?, remote: r.get(4)? })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// One `(local_id, root)` per distinct `common_dir`; a project without one is its own group.
    /// The pick is the earliest registered whose root still exists, else the earliest registered.
    pub fn repos_to_poll(&self) -> Result<Vec<(String, String)>, StoreError> {
        let mut groups: Vec<(Option<String>, Vec<ProjectRow>)> = Vec::new();
        for p in self.projects()? {
            match p.common_dir.as_ref().and_then(|c| groups.iter().position(|(k, _)| k.as_ref() == Some(c))) {
                Some(i) => groups[i].1.push(p),
                None => groups.push((p.common_dir.clone(), vec![p])),
            }
        }
        Ok(groups
            .into_iter()
            .map(|(_, ps)| {
                let pick = ps.iter().find(|p| Path::new(&p.root).exists()).unwrap_or(&ps[0]);
                (pick.local_id.clone(), pick.root.clone())
            })
            .collect())
    }

    /// `project` plus every project sharing its `common_dir`.
    pub fn project_group(&self, project: &str) -> Result<Vec<String>, StoreError> {
        let mut st = self.conn.prepare(
            "SELECT local_id FROM projects WHERE common_dir IS NOT NULL
             AND common_dir = (SELECT common_dir FROM projects WHERE local_id = ?1)",
        )?;
        let mut v: Vec<String> = st.query_map([project], |r| r.get(0))?.collect::<Result<_, _>>()?;
        if !v.iter().any(|p| p == project) {
            v.push(project.to_string());
        }
        Ok(v)
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
    /// `entries` still moves it. `scored_events` is how many events with `ts <= finalized_through`
    /// the caller scored; if the stored count differs, another writer added one meanwhile and
    /// nothing is written (`Conflict`). Events are never deleted, so equal counts mean equal sets.
    /// Does not check the skipped count; see `append_ledger_checked`.
    pub fn append_ledger(
        &mut self,
        entries: &[LedgerEntry],
        finalized_through: i64,
        scored_events: usize,
        skips: &[(String, i64, String)],
    ) -> Result<(), StoreError> {
        self.append_ledger_checked(entries, finalized_through, scored_events, skips, None)
    }

    /// Like `append_ledger`; with `Some(n)`, also `Conflict` if the stored skipped_claims
    /// count is not `n` (another finalizer recorded skips since the caller read them).
    pub fn append_ledger_checked(
        &mut self,
        entries: &[LedgerEntry],
        finalized_through: i64,
        scored_events: usize,
        skips: &[(String, i64, String)],
        skipped_seen: Option<usize>,
    ) -> Result<(), StoreError> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let stored: i64 = tx.query_row("SELECT COUNT(*) FROM events WHERE ts <= ?1", [finalized_through], |r| r.get(0))?;
        if usize::try_from(stored).ok() != Some(scored_events) {
            return Err(StoreError::Conflict);
        }
        if let Some(n) = skipped_seen {
            let have: i64 = tx.query_row("SELECT COUNT(*) FROM skipped_claims", [], |r| r.get(0))?;
            if usize::try_from(have).ok() != Some(n) {
                return Err(StoreError::Conflict);
            }
        }
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
        for (id, ts, reason) in skips {
            tx.execute(
                "INSERT OR IGNORE INTO skipped_claims(action_id, ts, reason) VALUES (?1, ?2, ?3)",
                params![id, ts, reason],
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

    /// Monotone fact table: each span extends the row with the greatest `end_ts` if it
    /// overlaps or touches it, else becomes a new row. Rows never shrink.
    pub fn extend_coverage(&mut self, spans: &[Interval]) -> Result<(), StoreError> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        for &(s, e) in spans.iter().filter(|(s, e)| s <= e) {
            let last: Option<(i64, i64)> = tx
                .query_row("SELECT rowid, end_ts FROM coverage ORDER BY end_ts DESC LIMIT 1", [], |r| Ok((r.get(0)?, r.get(1)?)))
                .optional()?;
            match last {
                Some((rowid, end)) if s <= end => {
                    tx.execute("UPDATE coverage SET end_ts = MAX(end_ts, ?1) WHERE rowid = ?2", params![e, rowid])?;
                }
                _ => {
                    tx.execute("INSERT INTO coverage(start_ts, end_ts) VALUES (?1, ?2)", params![s, e])?;
                }
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn coverage(&self) -> Result<Vec<Interval>, StoreError> {
        let mut st = self.conn.prepare("SELECT start_ts, end_ts FROM coverage ORDER BY start_ts")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn set_presence_required(&mut self, on: bool) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO meta(key, value) VALUES ('presence_required', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [i64::from(on)],
        )?;
        Ok(())
    }

    pub fn presence_required(&self) -> Result<bool, StoreError> {
        let v: Option<i64> = self
            .conn
            .query_row("SELECT value FROM meta WHERE key = 'presence_required'", [], |r| r.get(0))
            .optional()?;
        Ok(v.unwrap_or(0) != 0)
    }

    /// Recorded skips `(action_id, ts, reason)` in ts order.
    pub fn skipped(&self) -> Result<Vec<(String, i64, String)>, StoreError> {
        let mut st = self.conn.prepare("SELECT action_id, ts, reason FROM skipped_claims ORDER BY ts, action_id")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
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

    /// The `hash` column of each ledger row, in append order (the chain tip `append_ledger` extends).
    pub fn ledger_hash_columns(&self) -> Result<Vec<String>, StoreError> {
        let mut st = self.conn.prepare("SELECT hash FROM ledger ORDER BY seq")?;
        let rows = st.query_map([], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Main ledger with its `seq`, in append order.
    pub fn ledger_rows(&self) -> Result<Vec<(i64, LedgerEntry)>, StoreError> {
        let mut st = self.conn.prepare("SELECT seq, entry_json FROM ledger ORDER BY seq")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
        let mut out = Vec::new();
        for r in rows {
            let (seq, j) = r?;
            out.push((seq, serde_json::from_str(&j)?));
        }
        Ok(out)
    }

    /// All chains of one project, in append order.
    pub fn published_rows(&self, project: &str) -> Result<Vec<PublishedRow>, StoreError> {
        let mut st = self
            .conn
            .prepare("SELECT main_seq, entry_json FROM published_ledger WHERE project = ?1 ORDER BY rowid")?;
        let rows = st.query_map([project], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
        let mut out = Vec::new();
        for r in rows {
            let (main_seq, j) = r?;
            out.push(PublishedRow { main_seq, entry: serde_json::from_str(&j)? });
        }
        Ok(out)
    }

    /// Distinct published project names, sorted.
    pub fn published_projects(&self) -> Result<Vec<String>, StoreError> {
        let mut st = self.conn.prepare("SELECT DISTINCT project FROM published_ledger ORDER BY project")?;
        let rows = st.query_map([], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// One transaction. Each row extends its `(project, ruleset)` chain (`prev_hash` = last stored
    /// hash, `GENESIS_HASH` when empty) or the whole call fails with `ChainBroken`; `project: None`
    /// is `ChainBroken` too. A duplicate `(project, main_seq)` is a constraint error; nothing is written.
    pub fn append_published(&mut self, rows: &[PublishedRow]) -> Result<(), StoreError> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        for row in rows {
            let e = &row.entry;
            let project = e.project.as_deref().ok_or(StoreError::ChainBroken)?;
            let last: Option<(i64, String)> = tx
                .query_row(
                    "SELECT n, hash FROM published_ledger WHERE project = ?1 AND ruleset = ?2 ORDER BY n DESC LIMIT 1",
                    params![project, e.ruleset],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            let (n, prev) = match &last {
                Some((n, h)) => (n + 1, h.as_str()),
                None => (1, GENESIS_HASH),
            };
            if e.prev_hash != prev {
                return Err(StoreError::ChainBroken);
            }
            tx.execute(
                "INSERT INTO published_ledger(project, ruleset, n, main_seq, hash, entry_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![project, e.ruleset, n, row.main_seq, e.hash, serde_json::to_string(e)?],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn meta_get(&self, key: &str) -> Result<Option<i64>, StoreError> {
        Ok(self
            .conn
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .optional()?)
    }

    pub fn meta_set(&mut self, key: &str, value: i64) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO meta(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// Whether a stored `commit` event by the agent exists for `project` with `ts` in `[from_ts, to_ts]`.
    /// The `sha` is deliberately not consulted.
    pub fn has_agent_commit_marker(&self, project: &str, from_ts: i64, to_ts: i64) -> Result<bool, StoreError> {
        Ok(self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM events WHERE ts BETWEEN ?2 AND ?3
             AND json_extract(json, '$.kind') = 'commit' AND json_extract(json, '$.by') = 'agent'
             AND json_extract(json, '$.project') = ?1)",
            params![project, from_ts, to_ts],
            |r| r.get(0),
        )?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tally_scoring::event::Kind;
    use tally_scoring::{Event, LedgerEntry, GENESIS_HASH};

    #[test]
    fn agent_commit_marker_matches_by_agent_project_and_window_only() {
        let mut s = Store::open_in_memory().unwrap();
        let c = |id: &str, ts: i64, project: &str, by: tally_scoring::event::Actor| Event {
            project: Some(project.into()),
            kind: Kind::Commit { sha: id.into(), by, lines_changed: 0 },
            ..ev(id, ts)
        };
        use tally_scoring::event::Actor;
        s.append_events(&[c("claude:toolu_1", 1000, "p", Actor::Agent), c("u", 5000, "p", Actor::User)]).unwrap();
        assert!(s.has_agent_commit_marker("p", 900, 1100).unwrap());
        assert!(!s.has_agent_commit_marker("p", 1001, 1100).unwrap());
        assert!(!s.has_agent_commit_marker("q", 900, 1100).unwrap());
        assert!(!s.has_agent_commit_marker("p", 4900, 5100).unwrap());
    }

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

    fn pubentry(project: &str, ruleset: &str, action_id: &str, prev: &str) -> LedgerEntry {
        let mut e = entry(action_id, prev);
        e.project = Some(project.into());
        e.ruleset = ruleset.into();
        e.hash = tally_scoring::ledger::entry_hash(&e);
        e
    }
    #[test] fn published_chains_are_per_project_and_ruleset_and_start_at_genesis() {
        let mut s = Store::open_in_memory().unwrap();
        let a1 = pubentry("o/r", "r1", "a1", GENESIS_HASH);
        let a2 = pubentry("o/r", "r1", "a2", &a1.hash);
        let b1 = pubentry("o/r", "r2", "b1", GENESIS_HASH);
        s.append_published(&[PublishedRow { main_seq: 1, entry: a1.clone() },
                             PublishedRow { main_seq: 2, entry: a2.clone() },
                             PublishedRow { main_seq: 3, entry: b1.clone() }]).unwrap();
        assert_eq!(s.published_rows("o/r").unwrap().len(), 3);
        assert_eq!(s.published_projects().unwrap(), ["o/r"]);
    }
    #[test] fn a_broken_or_duplicate_published_append_writes_nothing() {
        let mut s = Store::open_in_memory().unwrap();
        let a1 = pubentry("o/r", "r1", "a1", GENESIS_HASH);
        s.append_published(&[PublishedRow { main_seq: 1, entry: a1.clone() }]).unwrap();
        let bad = pubentry("o/r", "r1", "a2", GENESIS_HASH);
        assert!(matches!(s.append_published(&[PublishedRow { main_seq: 2, entry: bad }]), Err(StoreError::ChainBroken)));
        let dup = pubentry("o/r", "r1", "a2", &a1.hash);
        assert!(s.append_published(&[PublishedRow { main_seq: 1, entry: dup }]).is_err());
        assert_eq!(s.published_rows("o/r").unwrap().len(), 1);
    }
    #[test] fn published_rows_cannot_be_updated_or_deleted() {
        let mut s = Store::open_in_memory().unwrap();
        s.append_published(&[PublishedRow { main_seq: 1, entry: pubentry("o/r", "r1", "a1", GENESIS_HASH) }]).unwrap();
        assert!(s.conn.execute("UPDATE published_ledger SET hash = 'x'", []).is_err());
        assert!(s.conn.execute("DELETE FROM published_ledger", []).is_err());
    }
    #[test] fn meta_roundtrip_and_ledger_rows_carry_seq() {
        let mut s = Store::open_in_memory().unwrap();
        assert_eq!(s.meta_get("k").unwrap(), None);
        s.meta_set("k", 7).unwrap(); s.meta_set("k", 8).unwrap();
        assert_eq!(s.meta_get("k").unwrap(), Some(8));
        s.append_ledger(&[entry("a1", GENESIS_HASH)], 10, 0, &[]).unwrap();
        assert_eq!(s.ledger_rows().unwrap()[0].0, 1);
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
        s.append_ledger(&[], 1000, 1, &[]).unwrap();
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
    #[test] fn only_config_disabled_sources_are_skipped_by_the_horizon() {
        let mut s = Store::open_in_memory().unwrap();
        s.set_watermark("git", 100).unwrap(); s.set_watermark("presence", 5).unwrap();
        s.set_watermark("claude-code", 7).unwrap(); s.set_watermark("ingest", 50).unwrap();
        assert_eq!(s.horizon().unwrap(), Some(5));                 // key absent: every row counts
        s.set_disabled_sources(&["presence", "claude-code"]).unwrap();
        assert_eq!(s.horizon().unwrap(), Some(50));                // ingest still counts
        s.set_disabled_sources(&["claude-code"]).unwrap();         // presence re-enabled at the next start
        assert_eq!(s.horizon().unwrap(), Some(5));
        s.set_disabled_sources(&[]).unwrap();
        assert_eq!(s.horizon().unwrap(), Some(5));
    }
    #[test] fn run_config_sets_presence_required_and_the_disabled_list_together() {
        let mut s = Store::open_in_memory().unwrap();
        s.set_watermark("presence", 5).unwrap(); s.set_watermark("git", 9).unwrap();
        s.set_run_config(true, &["presence"]).unwrap();
        assert!(s.presence_required().unwrap());
        assert_eq!(s.horizon().unwrap(), Some(9));
        s.set_run_config(false, &[]).unwrap();
        assert!(!s.presence_required().unwrap());
        assert_eq!(s.horizon().unwrap(), Some(5));
    }
    #[test] fn a_skipped_row_between_prepare_and_append_is_a_conflict() {
        let mut s = Store::open_in_memory().unwrap();
        s.append_ledger(&[], 10, 0, &[("a".into(), 5, "r".into())]).unwrap();
        let e = entry("a1", GENESIS_HASH);
        assert!(matches!(s.append_ledger_checked(std::slice::from_ref(&e), 20, 0, &[], Some(0)), Err(StoreError::Conflict)));
        assert!(s.ledger().unwrap().is_empty());
        assert_eq!(s.finalized_through().unwrap(), 10);
        s.append_ledger_checked(&[e], 20, 0, &[], Some(1)).unwrap();
        assert_eq!(s.ledger().unwrap().len(), 1);
    }
    #[test] fn ledger_appends_in_order_and_rejects_a_broken_link() {
        let mut s = Store::open_in_memory().unwrap();
        let e1 = entry("a1", GENESIS_HASH);
        let e2 = entry("a2", &e1.hash);
        s.append_ledger(&[e1.clone(), e2.clone()], 10, 0, &[]).unwrap();
        assert_eq!(s.ledger().unwrap(), vec![e1, e2]);
        assert!(matches!(s.append_ledger(&[entry("a3", GENESIS_HASH)], 20, 0, &[]), Err(StoreError::ChainBroken)));
        assert_eq!(s.ledger().unwrap().len(), 2);
        assert_eq!(s.finalized_through().unwrap(), 10);
    }
    #[test] fn ledger_rows_cannot_be_updated_or_deleted() {
        let mut s = Store::open_in_memory().unwrap();
        s.append_ledger(&[entry("a1", GENESIS_HASH)], 10, 0, &[]).unwrap();
        assert!(s.conn.execute("UPDATE ledger SET hash = 'x'", []).is_err());
        assert!(s.conn.execute("DELETE FROM ledger", []).is_err());
    }
    #[test] fn two_handles_on_one_file_see_each_others_writes() {
        let dir = tempfile::tempdir().unwrap(); let p = dir.path().join("tally.db");
        let mut a = Store::open(&p).unwrap();
        let mut b = Store::open(&p).unwrap();
        b.append_ledger(&[entry("a1", GENESIS_HASH)], 1000, 0, &[]).unwrap();
        assert!(matches!(a.append_events(&[ev("late", 1000)]),
            Err(StoreError::LateEvent { ts: 1000, finalized_through: 1000, .. })));
        assert_eq!(a.ledger().unwrap().len(), 1);
    }
    #[test] fn an_event_ingested_under_the_horizon_mid_finalize_is_a_conflict() {
        let dir = tempfile::tempdir().unwrap(); let p = dir.path().join("tally.db");
        let mut a = Store::open(&p).unwrap();
        let mut b = Store::open(&p).unwrap();
        a.append_events(&[ev("x", 10)]).unwrap();
        let scored = a.events().unwrap().iter().filter(|e| e.ts <= 1000).count();
        b.append_events(&[ev("new_source", 500), ev("newer", 2000)]).unwrap();
        assert!(matches!(a.append_ledger(&[entry("a1", GENESIS_HASH)], 1000, scored, &[]), Err(StoreError::Conflict)));
        assert!(a.ledger().unwrap().is_empty());
        assert_eq!(a.finalized_through().unwrap(), i64::MIN);
        // events newer than the horizon never conflict
        a.append_ledger(&[], 1000, 2, &[]).unwrap();
        assert_eq!(a.finalized_through().unwrap(), 1000);
    }
    #[test] fn coverage_merges_touching_intervals_and_keeps_gaps() {
        let mut s = Store::open_in_memory().unwrap();
        s.extend_coverage(&[(0, 10), (10, 20), (30, 40), (35, 50)]).unwrap();
        assert_eq!(s.coverage().unwrap(), vec![(0, 20), (30, 50)]);
        s.extend_coverage(&[(5, 8), (50, 60)]).unwrap();
        assert_eq!(s.coverage().unwrap(), vec![(0, 20), (30, 60)]);
    }
    #[test] fn skips_are_recorded_with_the_ledger_append_and_cannot_change() {
        let mut s = Store::open_in_memory().unwrap();
        s.append_ledger(&[], 10, 0, &[("a".into(), 5, "no_presence_coverage".into())]).unwrap();
        assert_eq!(s.skipped().unwrap(), vec![("a".to_string(), 5, "no_presence_coverage".to_string())]);
        assert!(s.conn.execute("UPDATE skipped_claims SET reason = 'x'", []).is_err());
        assert!(s.conn.execute("DELETE FROM skipped_claims", []).is_err());
    }
    #[test] fn presence_required_defaults_off_and_persists() {
        let mut s = Store::open_in_memory().unwrap();
        assert!(!s.presence_required().unwrap());
        s.set_presence_required(true).unwrap();
        assert!(s.presence_required().unwrap());
    }
    #[test] fn source_state_roundtrips_per_source_and_upserts() {
        let mut s = Store::open_in_memory().unwrap();
        let m = |kv: &[(&str, &str)]| kv.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<BTreeMap<_, _>>();
        s.set_state("a", &m(&[("f1", "10"), ("f2", "20")])).unwrap();
        s.set_state("b", &m(&[("f1", "99")])).unwrap();
        s.set_state("a", &m(&[("f1", "11")])).unwrap();
        assert_eq!(s.get_state("a").unwrap(), m(&[("f1", "11"), ("f2", "20")]));
        assert_eq!(s.get_state("b").unwrap(), m(&[("f1", "99")]));
        assert!(s.get_state("none").unwrap().is_empty());
    }
    #[test] fn server_ids_group_by_public_name_and_never_touch_the_name() {
        let mut s = Store::open_in_memory().unwrap();
        s.register_project("id1", "C:/r", Some("owner/repo")).unwrap();
        s.register_project("id2", "C:/r2", Some("owner/repo")).unwrap();   // a worktree
        s.register_project("id3", "C:/q", None).unwrap();                  // unnamed
        s.set_server_id("id1", "srv_1").unwrap();
        s.set_server_id("id2", "srv_1").unwrap();
        s.set_server_id("id3", "srv_9").unwrap();
        assert_eq!(s.server_ids().unwrap(), BTreeMap::from([("owner/repo".to_string(), vec!["srv_1".to_string()])]));
        assert_eq!(s.project_names().unwrap().get("id1").map(String::as_str), Some("owner/repo"));
        assert!(matches!(s.set_server_id("nope", "x"), Err(StoreError::UnknownProject(_))));
    }
    #[test] fn an_old_database_without_server_id_is_migrated() {
        let dir = tempfile::tempdir().unwrap(); let p = dir.path().join("tally.db");
        { let c = rusqlite::Connection::open(&p).unwrap();
          c.execute_batch("CREATE TABLE projects(local_id TEXT PRIMARY KEY, root TEXT NOT NULL, public_name TEXT);
                           INSERT INTO projects VALUES ('id1','C:/r','owner/repo');").unwrap(); }
        let mut s = Store::open(&p).unwrap();
        s.set_server_id("id1", "srv_1").unwrap();
        drop(s);
        Store::open(&p).unwrap();                       // second open: no "duplicate column" error
    }
    #[test] fn file_store_uses_wal() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open(&dir.path().join("tally.db")).unwrap();
        let mode: String = s.conn.query_row("PRAGMA journal_mode", [], |r| r.get(0)).unwrap();
        assert_eq!(mode, "wal");
    }
    #[test] fn data_survives_reopen_and_projects_are_named() {
        let dir = tempfile::tempdir().unwrap(); let p = dir.path().join("tally.db");
        { let mut s = Store::open(&p).unwrap();
          s.append_events(&[ev("a", 1)]).unwrap();
          s.register_project("id1", "C:/r", Some("owner/repo")).unwrap();
          s.register_project("id2", "C:/q", None).unwrap();
          s.register_project("id1", "C:/r", None).unwrap(); }
        let mut s = Store::open(&p).unwrap();
        assert_eq!(s.events().unwrap().len(), 1);
        assert_eq!(s.project_names().unwrap().get("id1").map(String::as_str), Some("owner/repo"));
        assert_eq!(s.project_names().unwrap().len(), 1);
        s.register_project("id1", "C:/r", Some("owner/renamed")).unwrap();
        assert_eq!(s.project_names().unwrap().get("id1").map(String::as_str), Some("owner/renamed"));
    }
}
