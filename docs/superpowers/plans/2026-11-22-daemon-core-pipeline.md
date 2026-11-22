# Daemon Core Pipeline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A `tally` binary (crate `tally-daemon`) that stores events locally, scores them with `tally-scoring`, appends final claims to a local hash-chained ledger, and offers `ingest`, `project add`, `finalize`, `status`, and `verify` commands. It has no log adapters yet; events arrive as NDJSON.

**Architecture:** A library (`store`, `project`, `finalize`, `status`) plus a thin `clap` binary. SQLite (one file) holds events, per-source watermarks, a project registry, and the ledger. A claim is appended to the ledger only when every source's watermark proves no later event can change it, and events at or before the finalized point are rejected, so the chain is append-only.

**Tech Stack:** Rust stable (edition 2021), `tally-scoring` (path dependency), `rusqlite` (feature `bundled`), `serde_json`, `sha2`, `hex`, `clap` (derive), `dirs`, `anyhow`, `thiserror`, `tempfile` (dev).

**Spec:** `docs/superpowers/specs/2026-11-22-daemon-roadmap-design.md` (section 1), `docs/spec/scoring.md` (ruleset `2026.11-0`, §1 project id, §2 events, §6 ledger). Read both before every task.

## Global Constraints

- Reuse `tally-scoring`: `score`, `chain`, `verify_chain`, `LedgerEntry`, `Event`, `GENESIS_HASH`, `RULESET_VERSION`, and `ruleset::INTERRUPT_FOLLOW_MS`. Do not copy constants or reimplement scoring or hashing.
- Local project id = lowercase hex SHA-256 of the canonical absolute git-root path (spec §1). Canonical form: filesystem-canonicalized, Windows verbatim prefix `\\?\` removed, `\` replaced by `/`, trailing `/` removed, case left as is.
- No event text: the store keeps only what an `Event` contains (hashes and numbers, spec §2). No prompt text, file contents, commands, window titles or URLs may be written to disk by this crate.
- Ledger is append-only: no code path updates or deletes a ledger row. The ledger uses `RULESET_VERSION` from `tally-scoring`.
- Source contract (document it in `store.rs`): a source calls `set_watermark(source, ts)` to promise that it will never again emit an event with `ts <= watermark`, including span events (`idle`, `agent_busy`) whose start `ts` is at or below it, so a source with an open span keeps its watermark below the span's start until it emits the closed span.
- Finalization rule: a claim is final iff `claim.ts + INTERRUPT_FOLLOW_MS <= horizon`, where `horizon` is the minimum watermark over all sources that have one. No sources means nothing is final.
- All time values are `i64` milliseconds. No floats.
- Windows first, but everything compiles and tests pass on Windows, macOS and Linux. Default data directory is `dirs::data_local_dir()/tally`; the `TALLY_HOME` environment variable overrides it.
- The crate is `license = "AGPL-3.0-only"`, edition 2021, binary name `tally`.

## Review Focus

1. **Event arrives at or before the finalized point.** Expected: the whole batch is rejected with `StoreError::LateEvent`, nothing is written, the ledger is untouched. A duplicate of an already stored id is not late; it is ignored. Test in Task 2.
2. **Re-ingesting the same file, or finalizing twice.** Expected: no duplicate events, no new ledger entries, identical ledger. Test in Tasks 2 and 3.
3. **Several sources with different watermarks, and the exact boundary.** Expected: horizon is the minimum; a claim at exactly `horizon - INTERRUPT_FOLLOW_MS` is final, one millisecond later is not; no watermark at all means nothing is final. Test in Task 3.
4. **Ledger no longer matches a fresh scoring run** (an event that bypassed the late check, or a changed scoring rule). Expected: `finalize` returns `FinalizeError::Divergence` and appends nothing. Test in Task 3.
5. **Project paths.** Expected: Windows verbatim prefixes, backslashes and trailing separators normalize to the same id; a directory with no `.git` ancestor gives `None`; a worktree's `.git` file counts as a git root. Test in Task 1.

## File Structure

```
Cargo.toml                           add "crates/tally-daemon" to workspace members
crates/tally-daemon/
  Cargo.toml                         binary `tally`, library `tally_daemon`
  src/lib.rs                         module wiring + re-exports
  src/project.rs                     normalize_path, project_id, find_git_root
  src/store.rs                       Store: SQLite schema, events, watermarks, projects, ledger rows
  src/finalize.rs                    finalize(): score, select final claims, chain, append
  src/status.rs                      status() and verify_ledger()
  src/main.rs                        clap CLI wiring only
  tests/cli.rs                       end-to-end test driving the real binary
.github/workflows/daemon.yml         cargo test -p tally-daemon on three OSes
```

---

### Task 1: Crate skeleton and project identity

**Files:**
- Create: `crates/tally-daemon/Cargo.toml`, `crates/tally-daemon/src/lib.rs`, `crates/tally-daemon/src/project.rs`, `crates/tally-daemon/src/main.rs` (placeholder `fn main() {}` only)
- Modify: `Cargo.toml` (workspace `members`)

**Interfaces:**
- Produces (`project.rs`, re-exported from `lib.rs`):
  - `pub fn normalize_path(raw: &str) -> String`: removes a leading `\?\`, replaces `\` with `/`, removes a trailing `/` unless the result is `/` or a drive root such as `C:/`.
  - `pub fn project_id_from_normalized(normalized: &str) -> String`: lowercase hex SHA-256 of the UTF-8 bytes.
  - `pub fn project_id(root: &std::path::Path) -> std::io::Result<String>`: `std::fs::canonicalize`, then `normalize_path`, then `project_id_from_normalized`.
  - `pub fn find_git_root(start: &std::path::Path) -> Option<std::path::PathBuf>`: `start` and each ancestor in turn; the first one containing a `.git` entry (directory, or file as in a worktree) is the root.
- `Cargo.toml` of the crate: `[lib] name = "tally_daemon"`, `[[bin]] name = "tally"`, dependencies `tally-scoring = { path = "../tally-scoring" }`, `sha2 = "0.10"`, `hex = "0.4"`; dev-dependency `tempfile = "3"`. More dependencies are added by the task that needs them.

- [ ] **Step 1: Write the failing tests** in `project.rs`:

```rust
#[test]
fn normalize_handles_windows_and_trailing_separators() {
    assert_eq!(normalize_path(r"\?\C:\Users\o\tally\"), "C:/Users/o/tally");
    assert_eq!(normalize_path(r"C:\Users\o\tally"), "C:/Users/o/tally");
    assert_eq!(normalize_path("/home/o/tally/"), "/home/o/tally");
    assert_eq!(normalize_path("/"), "/");
    assert_eq!(normalize_path(r"C:\"), "C:/");
}
#[test]
fn project_id_is_sha256_of_normalized_path() {
    assert_eq!(
        project_id_from_normalized("C:/Users/o/tally"),
        "e72dd4b784b9b9080795c60969a3bfeaa51b8f6efe4b78cd6667db6625f36965"
    );
}
#[test]
fn git_root_found_from_subdirectory_and_for_worktree_file() {
    let t = tempfile::tempdir().unwrap();
    let sub = t.path().join("a").join("b");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::create_dir(t.path().join(".git")).unwrap();
    assert_eq!(find_git_root(&sub).unwrap(), t.path());
    let w = tempfile::tempdir().unwrap();
    std::fs::write(w.path().join(".git"), "gitdir: elsewhere").unwrap();
    assert_eq!(find_git_root(w.path()).unwrap(), w.path());
}
#[test]
fn no_git_root_in_plain_directory() {
    let t = tempfile::tempdir().unwrap();
    // an ancestor of the temp dir could in principle be a repo; only a root inside it would be wrong
    assert!(find_git_root(t.path()).map_or(true, |r| !r.starts_with(t.path())));
}
```

- [ ] **Step 2:** Run `cargo test -p tally-daemon`. Expected: FAIL (functions not defined).
- [ ] **Step 3:** Implement the four functions to the interfaces above. `project_id` must canonicalize before normalizing.
- [ ] **Step 4:** Run `cargo test -p tally-daemon`. Expected: PASS, no warnings.
- [ ] **Step 5:** Commit: `feat(daemon): crate skeleton and project identity`.

---

### Task 2: Store

**Files:**
- Create: `crates/tally-daemon/src/store.rs`
- Modify: `crates/tally-daemon/Cargo.toml` (add `rusqlite = { version = "0.32", features = ["bundled"] }`, `serde_json = "1"`, `thiserror = "1"`), `crates/tally-daemon/src/lib.rs` (`pub mod store;`)

**Interfaces:**
- Consumes: `tally_scoring::{Event, LedgerEntry, GENESIS_HASH}` (both structs are serde-serializable).
- Produces (`store.rs`):
  - `pub struct Store` (owns a `rusqlite::Connection`), `pub struct AppendSummary { pub added: usize, pub duplicates: usize }`
  - `#[derive(thiserror::Error, Debug)] pub enum StoreError { Sqlite(rusqlite::Error), Json(serde_json::Error), LateEvent { id: String, ts: i64, finalized_through: i64 }, ChainBroken }` (`#[from]` on the first two)
  - `Store::open(path: &Path) -> Result<Store, StoreError>` (creates the file and schema), `Store::open_in_memory() -> Result<Store, StoreError>`
  - `append_events(&mut self, events: &[Event]) -> Result<AppendSummary, StoreError>`: one transaction. Per event: an `id` already stored is a duplicate and is skipped (checked first, even if its `ts` is late); otherwise `ts <= finalized_through()` fails the whole batch with `LateEvent`; otherwise insert.
  - `events(&self) -> Result<Vec<Event>, StoreError>` sorted by `(ts, id)`.
  - `set_watermark(&mut self, source: &str, ts: i64)`: keeps the larger of the old and new value. `horizon(&self) -> Result<Option<i64>, StoreError>`: minimum over all sources, `None` if there are none.
  - `finalized_through(&self) -> Result<i64, StoreError>`: `i64::MIN` until the first `append_ledger`.
  - `register_project(&mut self, local_id: &str, root: &str, public_name: Option<&str>)` (upsert) and `project_names(&self) -> Result<BTreeMap<String, String>, StoreError>` (local id to public name, only projects that have a name).
  - `append_ledger(&mut self, entries: &[LedgerEntry], finalized_through: i64) -> Result<(), StoreError>`: one transaction; fails with `ChainBroken` if `entries[0].prev_hash` is not the last stored hash (`GENESIS_HASH` when empty); stores each entry's JSON; sets `finalized_through` to the larger of the old value and the argument (an empty `entries` still moves it). `ledger(&self) -> Result<Vec<LedgerEntry>, StoreError>` in append order.
  - Schema: `events(id TEXT PRIMARY KEY, ts INTEGER NOT NULL, json TEXT NOT NULL)` with an index on `ts`; `watermarks(source TEXT PRIMARY KEY, ts INTEGER NOT NULL)`; `projects(local_id TEXT PRIMARY KEY, root TEXT NOT NULL, public_name TEXT)`; `ledger(seq INTEGER PRIMARY KEY, hash TEXT NOT NULL, entry_json TEXT NOT NULL)`; `meta(key TEXT PRIMARY KEY, value INTEGER NOT NULL)`; plus triggers that `RAISE(ABORT, ...)` on `UPDATE` and `DELETE` of `ledger`.

- [ ] **Step 1: Write the failing tests** in `store.rs`. Helpers: `ev(id, ts)` builds a `Kind::Interrupt` event with those values (`v:1, source:"test", project:None, session:None`); `entry(action_id, prev_hash)` builds a `LedgerEntry` (any field values) and sets `hash` with `tally_scoring::ledger::entry_hash`.

```rust
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
    assert_eq!(s.events().unwrap().len(), 1);              // "ok" was not written either
    let r = s.append_events(&[ev("old", 500), ev("ok", 1001)]).unwrap();
    assert_eq!((r.added, r.duplicates), (1, 1));           // old duplicate is fine, not late
}
#[test] fn horizon_is_the_minimum_watermark_and_watermarks_never_move_back() {
    let mut s = Store::open_in_memory().unwrap();
    assert_eq!(s.horizon().unwrap(), None);
    s.set_watermark("a", 100).unwrap(); s.set_watermark("b", 50).unwrap();
    assert_eq!(s.horizon().unwrap(), Some(50));
    s.set_watermark("b", 10).unwrap();                     // ignored
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
    assert_eq!(s.finalized_through().unwrap(), 10);        // failed append did not move it
}
#[test] fn ledger_rows_cannot_be_updated_or_deleted() {
    let mut s = Store::open_in_memory().unwrap();
    s.append_ledger(&[entry("a1", GENESIS_HASH)], 10).unwrap();
    assert!(s.conn.execute("UPDATE ledger SET hash = 'x'", []).is_err());
    assert!(s.conn.execute("DELETE FROM ledger", []).is_err());
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
```

- [ ] **Step 2:** Run `cargo test -p tally-daemon store`. Expected: FAIL (type not defined).
- [ ] **Step 3:** Implement `Store` to the interfaces above. Run every write inside a transaction; create the schema with `CREATE TABLE IF NOT EXISTS`; store events as `serde_json::to_string(&event)`. Put the source contract from Global Constraints in a doc comment on `set_watermark`.
- [ ] **Step 4:** Run `cargo test -p tally-daemon`. Expected: PASS, no warnings.
- [ ] **Step 5:** Commit: `feat(daemon): sqlite store for events, watermarks, projects and ledger`.

---

### Task 3: Finalize

**Files:**
- Create: `crates/tally-daemon/src/finalize.rs`
- Modify: `crates/tally-daemon/src/store.rs` (make the `conn` field `pub(crate)` so tests can bypass the late-event check), `crates/tally-daemon/src/lib.rs` (`pub mod finalize;`)

**Interfaces:**
- Consumes: `Store` (Task 2); `tally_scoring::{score, chain, Claim, GENESIS_HASH}`; `tally_scoring::ruleset::INTERRUPT_FOLLOW_MS`.
- Produces (`finalize.rs`):
  - `pub struct FinalizeSummary { pub appended: usize, pub final_total: usize, pub provisional: usize }` (`final_total` = ledger length after the call; `provisional` = claims from `score()` not yet final)
  - `#[derive(thiserror::Error, Debug)] pub enum FinalizeError { Store(#[from] StoreError), Divergence { index: usize, action_id: String } }`
  - `pub fn finalize(store: &mut Store) -> Result<FinalizeSummary, FinalizeError>`

Algorithm (the part the signature does not determine):
1. `claims = score(&store.events()?)`. Replace each claim's `project` when it is `Some(local_id)` and `store.project_names()?` has a public name for it. Unregistered ids stay as they are; `None` stays `None`.
2. `final_count` = number of leading claims with `ts + INTERRUPT_FOLLOW_MS <= horizon` (claims are in time order); 0 if `horizon()` is `None`.
3. Compare the stored ledger with `claims` pairwise over the stored length by `(action_id, ts, project, verified_s, unverified_s)`. A mismatch, or a ledger longer than `claims`, is `Divergence { index, action_id }`; append nothing.
4. `new = claims[ledger.len()..final_count]` (empty if `final_count <= ledger.len()`). Chain it from the last stored hash (`GENESIS_HASH` when empty) with `chain` and call `store.append_ledger(&entries, horizon)` whenever a horizon exists, even if `entries` is empty, so `finalized_through` follows the horizon.

- [ ] **Step 1: Write the failing tests** in `finalize.rs`. Helpers: `prompt(id, ts_ms, rel)` (`Kind::Prompt { chars: 200, relevance_pm: rel, novelty_bytes: 100 }`), `edit(id, ts_ms)` (`Kind::UserEdit`), both with `project: Some("p1")`, `source: "test"`.

```rust
#[test] fn final_prefix_boundary_is_exact() {
    let mut s = Store::open_in_memory().unwrap();
    s.append_events(&[prompt("a", 0, 600), edit("b", 100_000)]).unwrap();
    s.set_watermark("src", 219_999).unwrap();
    let r = finalize(&mut s).unwrap();
    assert_eq!((r.appended, r.final_total, r.provisional), (1, 1, 1));   // b needs 220_000
    s.set_watermark("src", 220_000).unwrap();
    let r = finalize(&mut s).unwrap();
    assert_eq!((r.appended, r.final_total, r.provisional), (1, 2, 0));
}
#[test] fn nothing_is_final_without_sources_and_horizon_is_the_slowest_source() {
    let mut s = Store::open_in_memory().unwrap();
    s.append_events(&[prompt("a", 0, 600)]).unwrap();
    assert_eq!(finalize(&mut s).unwrap().appended, 0);
    s.set_watermark("fast", 10_000_000).unwrap();
    s.set_watermark("slow", 100_000).unwrap();                 // 0 + 120_000 > 100_000
    assert_eq!(finalize(&mut s).unwrap().appended, 0);
    s.set_watermark("slow", 120_000).unwrap();
    assert_eq!(finalize(&mut s).unwrap().appended, 1);
}
#[test] fn finalizing_twice_changes_nothing() {
    let mut s = Store::open_in_memory().unwrap();
    s.append_events(&[prompt("a", 0, 600), edit("b", 100_000)]).unwrap();
    s.set_watermark("src", 1_000_000).unwrap();
    finalize(&mut s).unwrap();
    let first = s.ledger().unwrap();
    assert_eq!(finalize(&mut s).unwrap().appended, 0);
    assert_eq!(s.ledger().unwrap(), first);
}
#[test] fn incremental_finalization_equals_one_shot_and_chain_verifies() {
    let all = [prompt("a", 0, 600), edit("b", 100_000), prompt("c", 900_000, 600)];
    let mut inc = Store::open_in_memory().unwrap();
    inc.append_events(&all[..2]).unwrap(); inc.set_watermark("src", 300_000).unwrap(); finalize(&mut inc).unwrap();
    inc.append_events(&all[2..]).unwrap(); inc.set_watermark("src", 2_000_000).unwrap(); finalize(&mut inc).unwrap();
    let mut one = Store::open_in_memory().unwrap();
    one.append_events(&all).unwrap(); one.set_watermark("src", 2_000_000).unwrap(); finalize(&mut one).unwrap();
    assert_eq!(inc.ledger().unwrap(), one.ledger().unwrap());
    assert!(tally_scoring::verify_chain(&inc.ledger().unwrap(), GENESIS_HASH).is_ok());
}
#[test] fn project_ids_map_to_public_names_only_when_registered() {
    let mut s = Store::open_in_memory().unwrap();
    s.register_project("p1", "C:/r", Some("owner/repo")).unwrap();
    let mut other = edit("b", 100_000); other.project = Some("p2".into());
    let mut none = edit("c", 200_000);  none.project = None;
    s.append_events(&[prompt("a", 0, 600), other, none]).unwrap();
    s.set_watermark("src", 10_000_000).unwrap();
    finalize(&mut s).unwrap();
    let p: Vec<_> = s.ledger().unwrap().into_iter().map(|e| e.project).collect();
    assert_eq!(p, [Some("owner/repo".into()), Some("p2".into()), None]);
}
#[test] fn a_changed_past_is_a_divergence_and_appends_nothing() {
    let mut s = Store::open_in_memory().unwrap();
    s.append_events(&[prompt("a", 0, 600), prompt("b", 600_000, 400)]).unwrap();
    s.set_watermark("src", 1_000_000).unwrap();
    finalize(&mut s).unwrap();
    // bypass the late-event check: an idle span that would have shortened claim "b"
    let idle = Event { id: "i".into(), ts: 100_000, kind: Kind::Idle { end_ts: 590_000 }, ..prompt("x", 0, 0) };
    s.conn.execute("INSERT INTO events (id, ts, json) VALUES (?1, ?2, ?3)",
        rusqlite::params![idle.id, idle.ts, serde_json::to_string(&idle).unwrap()]).unwrap();
    let before = s.ledger().unwrap();
    assert!(matches!(finalize(&mut s), Err(FinalizeError::Divergence { index: 1, .. })));
    assert_eq!(s.ledger().unwrap(), before);
}
```

- [ ] **Step 2:** Run `cargo test -p tally-daemon finalize`. Expected: FAIL (function not defined).
- [ ] **Step 3:** Implement `finalize` per the algorithm above.
- [ ] **Step 4:** Run `cargo test -p tally-daemon`. Expected: PASS, no warnings.
- [ ] **Step 5:** Commit: `feat(daemon): finalize claims into the hash-chained ledger`.

---

### Task 4: Status and verify

**Files:**
- Create: `crates/tally-daemon/src/status.rs`
- Modify: `crates/tally-daemon/src/lib.rs` (`pub mod status;`)

**Interfaces:**
- Consumes: `Store` (Task 2), `finalize` (Task 3) in tests; `tally_scoring::{score, verify_chain, GENESIS_HASH}`.
- Produces (`status.rs`):
  - `pub struct Status { pub events: usize, pub final_claims: usize, pub provisional_claims: usize, pub verified_s: u64, pub unverified_s: u64, pub horizon: Option<i64>, pub finalized_through: Option<i64>, pub last_hash: String }` (`finalized_through` is `None` while the store holds `i64::MIN`; `last_hash` is `GENESIS_HASH` for an empty ledger; seconds are sums over ledger entries only; `provisional_claims` = `score(events).len()` minus ledger length, never negative)
  - `pub fn status(store: &Store) -> Result<Status, StoreError>`
  - `#[derive(thiserror::Error, Debug)] pub enum VerifyError { Store(#[from] StoreError), Chain { index: usize, reason: &'static str } }`
  - `pub fn verify_ledger(store: &Store) -> Result<usize, VerifyError>`: `Ok(n)` with the number of entries when `verify_chain(&ledger, GENESIS_HASH)` passes; otherwise `Chain` with the library's index and reason.

- [ ] **Step 1: Write the failing tests** in `status.rs`. The helper `store_with_two_final_claims()` appends `prompt("a", 0, 600)` and `edit("b", 100_000)` (same helper style as Task 3), sets watermark `1_000_000`, and calls `finalize`.

```rust
#[test] fn status_counts_final_and_provisional_claims() {
    let mut s = store_with_two_final_claims();
    s.append_events(&[edit("c", 2_000_000)]).unwrap();         // not final yet: watermark is 1_000_000
    let st = status(&s).unwrap();
    assert_eq!((st.events, st.final_claims, st.provisional_claims), (3, 2, 1));
    assert_eq!(st.verified_s, 220);                            // 120 (first action) + 100 (b)
    assert_eq!((st.horizon, st.finalized_through), (Some(1_000_000), Some(1_000_000)));
    assert_eq!(st.last_hash, s.ledger().unwrap()[1].hash);
}
#[test] fn empty_store_status() {
    let st = status(&Store::open_in_memory().unwrap()).unwrap();
    assert_eq!((st.events, st.final_claims, st.horizon, st.finalized_through), (0, 0, None, None));
    assert_eq!(st.last_hash, GENESIS_HASH);
}
#[test] fn verify_passes_on_an_untouched_ledger_and_names_the_tampered_entry() {
    let s = store_with_two_final_claims();
    assert_eq!(verify_ledger(&s).unwrap(), 2);
    // tamper: drop every trigger (they exist to stop this), then change entry 2's seconds
    let names: Vec<String> = s.conn.prepare("SELECT name FROM sqlite_master WHERE type='trigger'").unwrap()
        .query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
    for n in names { s.conn.execute(&format!("DROP TRIGGER {n}"), []).unwrap(); }
    s.conn.execute("UPDATE ledger SET entry_json = replace(entry_json, '\"verified_s\":100', '\"verified_s\":900') WHERE seq = 2", []).unwrap();
    assert!(matches!(verify_ledger(&s), Err(VerifyError::Chain { index: 1, reason: "bad_hash" })));
}
```

(`seq` starts at 1, so `seq = 2` is index 1. If the entry JSON does not contain `"verified_s":100` exactly, the replace is a no-op: confirm the test fails for the right reason before relying on it.)

- [ ] **Step 2:** Run `cargo test -p tally-daemon status`. Expected: FAIL (module missing).
- [ ] **Step 3:** Implement `status` and `verify_ledger` to the interfaces above.
- [ ] **Step 4:** Run `cargo test -p tally-daemon`. Expected: PASS, no warnings.
- [ ] **Step 5:** Commit: `feat(daemon): status and ledger verification`.

---

### Task 5: CLI and CI

**Files:**
- Create: `crates/tally-daemon/src/main.rs` (replace the placeholder), `crates/tally-daemon/tests/cli.rs`, `.github/workflows/daemon.yml`
- Modify: `crates/tally-daemon/Cargo.toml` (add `clap = { version = "4", features = ["derive"] }`, `dirs = "5"`, `anyhow = "1"`)

**Interfaces:**
- Consumes: everything above.
- Produces: the `tally` binary. Global option `--home <DIR>` (env `TALLY_HOME`, default `dirs::data_local_dir()/tally`); the database is `<home>/tally.db`, created with its directory if missing. Subcommands and their exact stdout:
  - `tally ingest <FILE> [--source <NAME>] [--watermark <MS>]`: reads one JSON `Event` per line (blank lines skipped; a bad line is an error naming its line number), `append_events`, then `set_watermark(NAME, MS)` where `NAME` defaults to `ingest` and `MS` defaults to the newest `ts` in the file (the file is taken to be complete up to its newest event; skipped when the file has no events and no `--watermark` is given). Prints `added <n>, duplicates <m>`. A `LateEvent` error exits non-zero with a message that contains `late event`.
  - `tally project add <PATH> [--name <PUBLIC_NAME>]`: `find_git_root(PATH)` (error `not inside a git repository` if `None`), `project_id`, `register_project`. Prints the local project id.
  - `tally finalize`: prints `appended <n>, final <total>, provisional <p>`; a `Divergence` error exits non-zero.
  - `tally status`: prints one `key: value` line each, in this order: `events`, `final_claims`, `provisional_claims`, `verified_s`, `unverified_s`, `horizon`, `finalized_through` (`none` when absent), `last_hash`.
  - `tally verify`: prints `ok: <n> entries` and exits 0, or prints `FAIL at entry <index>: <reason>` and exits 1.
- `main.rs` contains only argument parsing and printing; all logic stays in the library.

- [ ] **Step 1: Write the failing end-to-end tests** in `tests/cli.rs`. Run the real binary with `std::process::Command::new(env!("CARGO_BIN_EXE_tally")).arg("--home").arg(dir)`. Build the input file by serializing `tally_scoring::Event` values (ids `"1"`, `"2"`, `"3"`; times are seconds times 1000): a prompt (`chars: 200, relevance_pm: 600, novelty_bytes: 100`) at 0 s, `Idle { end_ts: 1_860_000 }` at 60 s, a second identical prompt at 1900 s.

```rust
#[test] fn lunch_scenario_end_to_end() {
    // ingest with --watermark 3000000: "added 3, duplicates 0"
    // finalize: "appended 2, final 2, provisional 0"
    // status: contains "final_claims: 2" and "verified_s: 660" (120 + 540, same numbers as the scoring golden test)
    // verify: exit 0, stdout "ok: 2 entries"
    // ingest the same file again: "added 0, duplicates 3"
    // ingest a file with one new prompt at 1000 s (ts <= finalized_through 3_000_000): non-zero exit, stderr contains "late event"
    // finalize again: "appended 0, final 2, provisional 0"
}
#[test] fn project_add_rejects_a_directory_outside_git_and_prints_the_id_inside_one() {
    // a tempdir with a `.git` directory: exit 0, stdout is a 64-char lowercase hex string
    // a tempdir without one: non-zero exit, stderr contains "not inside a git repository"
    // (skip the negative half if an ancestor of the temp dir is itself a git repository)
}
#[test] fn a_bad_event_line_names_its_line_number() {
    // file with a valid event on line 1 and "not json" on line 2: non-zero exit, stderr contains "line 2"
}
```

- [ ] **Step 2:** Run `cargo test -p tally-daemon --test cli`. Expected: FAIL (binary does nothing).
- [ ] **Step 3:** Implement `main.rs` with `clap` derive and the exact outputs above. Use `anyhow` in `main.rs` only.
- [ ] **Step 4:** Run `cargo test -p tally-daemon`. Expected: all PASS, no warnings. Also run `cargo clippy -p tally-daemon --all-targets` and fix warnings.
- [ ] **Step 5:** Write `.github/workflows/daemon.yml`: on push and pull request, a matrix job (`ubuntu-latest`, `windows-latest`, `macos-latest`) with `actions/checkout@v4`, `dtolnay/rust-toolchain@stable`, `Swatinem/rust-cache@v2`, then `cargo test -p tally-daemon`.
- [ ] **Step 6:** Commit: `feat(daemon): tally CLI and CI`.

---

## Done when

- `cargo test -p tally-daemon` passes on all three operating systems in CI.
- The end-to-end test shows the lunch scenario giving 660 verified seconds through `ingest`, `finalize`, `status` and `verify`, matching the scoring crate's golden numbers.
- `tally verify` reports a changed ledger entry by index, and a late event is refused without touching the ledger.

## Not in this plan

A continuous loop and any log watching (section 2 adds the first source, which calls `set_watermark` and `finalize` on a timer), presence and git (section 3), publishing (section 4).
