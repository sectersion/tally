# Publish: Badge and Per-Project Ledger Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Once per local calendar day while `tally run` is running (and on demand with `tally publish`), the daemon publishes, for every project that has a registered public name, a badge endpoint JSON and a per-project hash chain to a dedicated orphan branch `tally-data` of the project's GitHub repository, using the user's own `git` authentication. The README shows the badge and tells readers how to verify the chain with `tally verify-published <file>`. Nothing about unnamed projects ever leaves the machine.

**Architecture:** Three new library modules. `published.rs`: derives per-project chains from the main ledger into a new append-only `published_ledger` table, checks the two ledgers agree, computes badge JSON, and verifies a downloaded chain file. `hosting.rs`: a thin wrapper over the `git` command that keeps a private clone of the `tally-data` branch under `<home>/publish/`, writes files, commits only when something changed, pushes, and refuses to overwrite a branch another machine has written. `publish.rs`: orchestrates one publish and decides when a daily publish is due. `finalize` gets a ruleset guard.

**Tech Stack:** Rust (existing crate `tally-daemon`), `tally-scoring` (`verify_chain`, `entry_hash`, `LedgerEntry`, `GENESIS_HASH`, `RULESET_VERSION`), `rusqlite`, `serde_json`, the system `git` binary (a runtime requirement for publishing only), `chrono` (feature `clock`, local calendar date). Dev: `tempfile`.

**Spec:** `docs/superpowers/specs/2026-11-22-daemon-roadmap-design.md` ("Badge and publishing", "Decided by the maintainer"), `docs/spec/scoring.md` §6 (ledger entry), `docs/superpowers/plans/README.md` (contracts C6 and C8).

## Global Constraints

- Reuse `tally-scoring`: `LedgerEntry`, `ledger::entry_hash`, `verify_chain`, `GENESIS_HASH`, `RULESET_VERSION`. Do not reimplement hashing or chain verification.
- Only claims already in the main ledger (final) are ever published. Publishing never calls `score()` and never reads events.
- A project is published only if it has a public name (`projects.public_name`). The name must match `^[A-Za-z0-9._-]+/[A-Za-z0-9._-]+$` and neither segment may be `.` or `..` (it becomes a path and a URL). An invalid name is an error that stops the publish; it is never silently skipped or sanitized.
- Published entries contain exactly the main-ledger fields (spec §6). Only `project` (public name), `prev_hash` and `hash` differ from the main entry. No local id, path, or other project's data is written to the clone, the commit, or any log line that is pushed.
- `published_ledger` is append-only (same `RAISE(ABORT)` triggers as `ledger`). One chain per `(project, ruleset)`; each chain starts at `GENESIS_HASH` and verifies alone.
- No tokens in config or in the database. Authentication is whatever `git push` already has (credential manager, `gh auth setup-git`, SSH). The daemon runs `git` with `GIT_TERMINAL_PROMPT=0` and `GCM_INTERACTIVE=never` so it can never hang on a prompt.
- Never force-push, never rewrite published history. The remote branch is only ever extended.
- Publishing by the daily job is off unless `[publish] enabled = true` in `<home>/config.toml` (single publisher, see Decisions). `tally publish` run by hand works regardless of `enabled`, so the first publish can be checked before turning the daily job on.
- Badge JSON (contract C8): `{"color":"blue","label":"tracked with Tally","message":"<N.N> h","schemaVersion":1}`, verified seconds only, all-time, summed over every published chain of the project, one decimal, rounded half up with integer arithmetic (no floats).
- A failure in publishing never stops `tally run`, never touches the main ledger or events, and is reported on stderr.
- Everything compiles and tests pass on Windows, macOS and Linux. Tests that need git use a local bare repository as the remote and return early (printing a note) if `git --version` fails.

## Review Focus

1. **A day with no new claims.** Expected: the daily job runs, derives no new rows, renders files byte-identical to the clone, `git status --porcelain` is empty, no commit, no push, and the day is still marked done. The badge is not rewritten. Tests in Tasks 5 and 7.
2. **The push failing halfway** (offline, auth revoked, remote rejects). Expected: local derivation (`published_ledger`) is already stored and harmless; the clone may hold an unpushed commit; the next run resets the clone to the remote branch, regenerates the same files and pushes. The remote never holds a badge without its ledger, because both files go in one commit and one push of one ref is atomic. The day is not marked done after a failure; the scheduler retries after at most one hour. Tests in Tasks 5 and 7.
3. **Two machines publishing the same project.** Expected: single publisher. Before writing, the daemon compares each remote chain file to its own derived chain; if the remote chain is not a prefix of the local one, it stops with `RemoteDiverged` and writes nothing. It never force-pushes. Test in Task 5.
4. **A ledger that does not verify.** Expected: before deriving anything, `verify_ledger` on the main ledger, `verify_chain` on every published chain, and a field-by-field consistency check between each published row and its main-ledger row. Any failure stops the publish naming the failing index; nothing is written or pushed. Tests in Tasks 3 and 5.
5. **Ruleset bump.** Expected: `finalize` refuses to append when the ledger's last entry has a ruleset other than `RULESET_VERSION` (Task 1); the published side never mixes rulesets in one chain (keyed by `(project, ruleset)`). Tests in Tasks 1 and 3.
6. **Project naming and renaming.** A name registered after claims were finalized under the local id still publishes those claims (mapped through the registry). A second local id (another worktree) with the same name joins the same chain. Renaming a published project is not supported: the new name starts a new chain from entries that map to it, and the old chain stays published. Test in Task 3.

## File Structure

```
crates/tally-daemon/
  Cargo.toml                    add chrono (feature clock)
  src/lib.rs                    pub mod published; pub mod hosting; pub mod publish;
  src/store.rs                  published_ledger table, ledger_rows, published_rows, append_published, meta_get/meta_set
  src/finalize.rs               RulesetChanged guard
  src/published.rs              derive/sync, consistency check, badge JSON, verify_published_json
  src/hosting.rs                git wrapper, clone management, push
  src/publish.rs                publish_once, PublishConfig, daily scheduling
  src/config.rs                 (plan 03) add `publish: PublishConfig`
  src/run.rs                    (plan 03) one `publish::tick` call per loop iteration
  src/main.rs                   `publish [--dry-run]`, `verify-published <FILE>...`
  tests/cli.rs                  end-to-end tests for both commands
README.md                       badge line and "How this badge is made"
```

## Decisions I made (maintainer to confirm)

1. **Hosting: an orphan branch `tally-data` of `sectersion/tally`, pushed with `git`.** Alternatives rejected: a Gist needs a token with the account-wide `gist` scope that cannot be limited to one repository, and is a second auth path; GitHub Pages adds a build step and a minutes-long delay on top of the same git push. The branch uses the credential the user already has for pushing (`gh auth setup-git`, Git Credential Manager, or an SSH key), which the user revokes in GitHub settings or with `gh auth logout`. A fine-grained token limited to this repository with Contents read and write is the recommended minimum. The branch history is the permanent public timestamp record the maintainer accepted.
2. **Layout on the branch:** `<owner>/<repo>/badge.json` and `<owner>/<repo>/ledger-<ruleset>.json` (a JSON array of entries, one entry per line, so appends are one-line diffs). The README badge URL is `https://img.shields.io/endpoint?url=<percent-encoded raw URL of badge.json>`.
3. **Caching:** `raw.githubusercontent.com` caches for about 5 minutes and shields.io caches endpoint badges for at least 5 minutes; GitHub's image proxy may add more. A badge that lags by roughly 10 to 15 minutes is expected and irrelevant for a daily job. The README note says so.
4. **Published chains are derived from main-ledger rows, not re-scored.** The published table stores `main_seq` (the main-ledger row an entry came from) so the two ledgers can be proven consistent, and so a name registered later still picks up older claims.
5. **Single publisher per project, enforced by a prefix check, not a lock.** `[publish] enabled` defaults to false; the machine that publishes is the one with it set. A second machine with the same name is refused (`RemoteDiverged`), not merged.
6. **Ruleset bump on the main ledger is guarded, not migrated.** `finalize` refuses to append across a ruleset change with an actionable error. Starting a new main chain needs a decision about re-scoring history that belongs to the plan that bumps the ruleset. The published side already supports it (chains are per ruleset), so that plan only has to change finalize.
7. **Failure retry:** a failed publish is retried at most once per hour; the day counts as done only after a successful publish (including a successful no-op).
8. **`tally publish` ignores `enabled`**; the daily job in `tally run` honors it.

## Assumes from earlier plans

- Plan 03: `crates/tally-daemon/src/config.rs` with a loader that reads `<home>/config.toml`, rejects unknown keys, and a `Config` struct to which each feature adds a table; `crates/tally-daemon/src/run.rs` with the `tally run` loop that polls sources, appends, sets watermarks and calls `finalize` once per iteration, with the current time in ms available per iteration. If names differ, adapt the two small edits in Tasks 6 and 7; nothing else depends on them.
- Plan 04: projects (worktrees and clones) register with the same public name through `register_project`; this plan only reads `project_names()`.
- Code on `main` today (verified): `Store` with `ledger()`, `project_names()`, `append_ledger(entries, finalized_through, scored_events)`; triggers `ledger_no_update` and `ledger_no_delete`; `meta(key TEXT PRIMARY KEY, value INTEGER)`; `verify_ledger(&Store)`; `main.rs` opens the store before dispatching every command (Task 4 changes that for `verify-published`). `finalize` replaces a claim's project with its public name only when registered at finalize time, so older main-ledger rows can carry the local id.

---

### Task 1: Ruleset guard in finalize

**Files:**
- Modify: `crates/tally-daemon/src/finalize.rs`

**Interfaces:**
- Produces: a new variant `FinalizeError::RulesetChanged { ledger: String, current: String }` with message `ledger was written under ruleset {ledger} but this build scores with {current}; finalize will not append across a ruleset change`.
- Rule: in `prepare`, after reading `ledger` and before the divergence loop, if the ledger is not empty and `ledger.last().ruleset != tally_scoring::RULESET_VERSION`, return `RulesetChanged`. Nothing is appended and `finalized_through` does not move. (Checking the last entry is enough: a ledger only ever ends in the newest ruleset it has seen.)

- [ ] **Step 1: Write the failing test** in `finalize.rs` tests. Build one entry by hand (the helper in `store.rs` tests is private, so repeat its four lines): `LedgerEntry { v:1, ruleset:"old-0".into(), action_id:"a".into(), ts:1, project:None, kind:"k".into(), verified_s:1, unverified_s:0, reasons:vec![], features:Default::default(), prev_hash:GENESIS_HASH.into(), hash:String::new() }`, then `e.hash = tally_scoring::ledger::entry_hash(&e)`.

```rust
#[test] fn finalize_refuses_to_append_across_a_ruleset_change() {
    let mut s = Store::open_in_memory().unwrap();
    s.append_ledger(&[old_ruleset_entry()], 5, 0).unwrap();
    s.append_events(&[prompt("a", 10, 600)]).unwrap();
    s.set_watermark("src", 10_000_000).unwrap();
    assert!(matches!(finalize(&mut s),
        Err(FinalizeError::RulesetChanged { ledger, .. }) if ledger == "old-0"));
    assert_eq!(s.ledger().unwrap().len(), 1);
    assert_eq!(s.finalized_through().unwrap(), 5);
}
```

- [ ] **Step 2:** Run `cargo test -p tally-daemon finalize_refuses`. Expected: FAIL (variant missing).
- [ ] **Step 3:** Add the variant and the check.
- [ ] **Step 4:** Run `cargo test -p tally-daemon`. Expected: PASS, no warnings. (Existing finalize tests build ledgers with the current ruleset, so they are unaffected.)
- [ ] **Step 5:** Commit: `feat(daemon): refuse to finalize across a ruleset change`.

---

### Task 2: published_ledger table

**Files:**
- Modify: `crates/tally-daemon/src/store.rs`

**Interfaces:**
- Schema (added to `SCHEMA`):

```sql
CREATE TABLE IF NOT EXISTS published_ledger(
  project TEXT NOT NULL, ruleset TEXT NOT NULL, n INTEGER NOT NULL,
  main_seq INTEGER NOT NULL, hash TEXT NOT NULL, entry_json TEXT NOT NULL,
  PRIMARY KEY(project, ruleset, n), UNIQUE(project, main_seq));
CREATE TRIGGER IF NOT EXISTS published_no_update BEFORE UPDATE ON published_ledger
BEGIN SELECT RAISE(ABORT, 'published ledger is append-only'); END;
CREATE TRIGGER IF NOT EXISTS published_no_delete BEFORE DELETE ON published_ledger
BEGIN SELECT RAISE(ABORT, 'published ledger is append-only'); END;
```

- Produces (`store.rs`):
  - `#[derive(Debug, Clone, PartialEq)] pub struct PublishedRow { pub main_seq: i64, pub entry: LedgerEntry }` (`entry.project` is `Some(public name)`, `entry.ruleset` is the chain's ruleset).
  - `Store::ledger_rows(&self) -> Result<Vec<(i64, LedgerEntry)>, StoreError>`: main ledger with its `seq`, in append order.
  - `Store::published_rows(&self, project: &str) -> Result<Vec<PublishedRow>, StoreError>`: all chains of one project, ordered by `rowid` (append order).
  - `Store::published_projects(&self) -> Result<Vec<String>, StoreError>`: distinct names, sorted.
  - `Store::append_published(&mut self, rows: &[PublishedRow]) -> Result<(), StoreError>`: one `Immediate` transaction; per row, `n` = 1 + the highest stored `n` of `(project, ruleset)`; the row's `prev_hash` must equal that chain's last stored hash (`GENESIS_HASH` when the chain is empty) or the whole call fails with `ChainBroken`. A duplicate `(project, main_seq)` is a `Sqlite` constraint error and writes nothing. A row with `project == None` is `ChainBroken`.
  - `Store::meta_get(&self, key: &str) -> Result<Option<i64>, StoreError>` and `Store::meta_set(&mut self, key: &str, value: i64) -> Result<(), StoreError>` (plain upsert on `meta`; used by Task 7 for `last_publish_day`).

- [ ] **Step 1: Write the failing tests** in `store.rs` tests (reuse the private `entry(action_id, prev_hash)` helper; add `fn pubentry(project, ruleset, action_id, prev)` that sets `project`/`ruleset` and recomputes `hash`):

```rust
#[test] fn published_chains_are_per_project_and_ruleset_and_start_at_genesis() {
    let mut s = Store::open_in_memory().unwrap();
    let a1 = pubentry("o/r", "r1", "a1", GENESIS_HASH);
    let a2 = pubentry("o/r", "r1", "a2", &a1.hash);
    let b1 = pubentry("o/r", "r2", "b1", GENESIS_HASH);       // new ruleset: new chain
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
    let bad = pubentry("o/r", "r1", "a2", GENESIS_HASH);       // not extending the chain
    assert!(matches!(s.append_published(&[PublishedRow { main_seq: 2, entry: bad }]), Err(StoreError::ChainBroken)));
    let dup = pubentry("o/r", "r1", "a2", &a1.hash);
    assert!(s.append_published(&[PublishedRow { main_seq: 1, entry: dup }]).is_err());   // same main_seq
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
    s.append_ledger(&[entry("a1", GENESIS_HASH)], 10, 0).unwrap();
    assert_eq!(s.ledger_rows().unwrap()[0].0, 1);
}
```

- [ ] **Step 2:** Run `cargo test -p tally-daemon published`. Expected: FAIL (items missing).
- [ ] **Step 3:** Implement to the interfaces. Reads use the same pattern as `ledger()`.
- [ ] **Step 4:** Run `cargo test -p tally-daemon`. Expected: PASS, no warnings.
- [ ] **Step 5:** Commit: `feat(daemon): append-only published_ledger table`.

---

### Task 3: Derive per-project chains, consistency check, badge

**Files:**
- Create: `crates/tally-daemon/src/published.rs`
- Modify: `crates/tally-daemon/src/lib.rs` (`pub mod published;`)

**Interfaces:**
- Consumes: `Store::{ledger_rows, project_names, published_rows, published_projects, append_published}` (Task 2); `tally_scoring::{verify_chain, GENESIS_HASH, LedgerEntry}`, `tally_scoring::ledger::entry_hash`; `status::verify_ledger`.
- Produces (`published.rs`):
  - `#[derive(thiserror::Error, Debug)] pub enum PublishError { Store(#[from] StoreError), MainLedger(#[from] VerifyError), BadName(String), Inconsistent { project: String, main_seq: i64, what: &'static str }, PublishedChain { project: String, ruleset: String, index: usize, reason: &'static str } }`
  - `pub fn valid_public_name(name: &str) -> bool` (rule in Global Constraints).
  - `pub fn pending(store: &Store) -> Result<Vec<PublishedRow>, PublishError>`: rows to append, no writes. Algorithm: (1) `names = project_names()`; every value must satisfy `valid_public_name` else `BadName(value)`. (2) For each main row `(seq, e)` with `e.project = Some(p)`: the public name is `names[p]` if `p` is a local id in `names`, else `p` itself if `p` equals some value in `names` (finalize already replaced it), else the row is not published. This is the only filter: unnamed projects and `project: None` never leave. (3) A row whose `(name, seq)` is already in `published_rows(name)` is skipped. (4) For the rest, in main-ledger order: `prev` = last hash of the `(name, e.ruleset)` chain counting rows already stored and rows built so far, else `GENESIS_HASH`; copy `e`, set `project = Some(name)`, `prev_hash = prev`, `hash = entry_hash(&copy)`.
  - `pub fn check(store: &Store) -> Result<(), PublishError>`: runs `verify_ledger(store)?`; then for every published project and every stored row: the main row with that `main_seq` exists and matches on `(action_id, ts, kind, verified_s, unverified_s, reasons, features, ruleset)` (else `Inconsistent { what: "main row differs" }` or `"main row missing"`), the entry's project is a valid public name that the registry maps to the main row's project (else `Inconsistent { what: "project mapping" }`), and each `(project, ruleset)` chain passes `verify_chain(.., GENESIS_HASH)` (else `PublishedChain`).
  - `pub fn sync(store: &mut Store, apply: bool) -> Result<Vec<PublishedRow>, PublishError>`: `check`, then `pending`; when `apply` is true, `append_published(&rows)`, retrying the whole body up to 3 attempts in total on `StoreError::ChainBroken` or a `Sqlite` constraint error (another process synced first). Returns the rows that were (or, with `apply = false`, would be) appended.
  - `pub fn verified_seconds(rows: &[PublishedRow]) -> u64` (sum of `entry.verified_s`).
  - `pub fn hours_message(verified_s: u64) -> String`: `tenths = (verified_s * 10 + 1800) / 3600`, formatted `"{tenths/10}.{tenths%10} h"`.
  - `pub fn badge_json(verified_s: u64) -> String`: `serde_json::to_string` of an object with `schemaVersion` 1, `label` "tracked with Tally", `message` = `hours_message`, `color` "blue", followed by a newline. serde_json sorts the keys, so the bytes are stable.

- [ ] **Step 1: Write the failing tests** in `published.rs`. Helper `store_with_ledger(name: Option<&str>)`: a store with `prompt("a", 0, 600)` and `edit("b", 100_000)` (from `test_support`, project `"p1"`), `register_project("p1", "C:/r", name)` when `name` is `Some`, watermark `1_000_000`, then `finalize`.

```rust
#[test] fn unnamed_projects_publish_nothing() {
    let mut s = store_with_ledger(None);
    assert!(sync(&mut s, true).unwrap().is_empty());
    assert!(s.published_projects().unwrap().is_empty());
}
#[test] fn a_named_project_gets_its_own_chain_that_verifies_alone() {
    let mut s = store_with_ledger(Some("owner/repo"));
    assert_eq!(sync(&mut s, true).unwrap().len(), 2);
    let entries: Vec<_> = s.published_rows("owner/repo").unwrap().into_iter().map(|r| r.entry).collect();
    assert!(entries.iter().all(|e| e.project.as_deref() == Some("owner/repo")));
    assert_eq!(entries[0].prev_hash, GENESIS_HASH);
    assert!(tally_scoring::verify_chain(&entries, GENESIS_HASH).is_ok());
    assert_ne!(entries[0].hash, s.ledger().unwrap()[0].hash);   // rechained, not copied
    assert!(sync(&mut s, true).unwrap().is_empty());             // idempotent
}
#[test] fn other_projects_in_the_main_ledger_never_appear() {
    let mut s = Store::open_in_memory().unwrap();
    let mut other = edit("b", 100_000); other.project = Some("p2".into());
    s.register_project("p1", "C:/r", Some("owner/repo")).unwrap();
    s.register_project("p2", "C:/q", None).unwrap();
    s.append_events(&[prompt("a", 0, 600), other]).unwrap();
    s.set_watermark("src", 1_000_000).unwrap();
    finalize(&mut s).unwrap();
    sync(&mut s, true).unwrap();
    let all = s.published_rows("owner/repo").unwrap();
    assert_eq!(all.len(), 1);
    assert!(!serde_json::to_string(&all).unwrap().contains("p2"));
}
#[test] fn a_name_registered_after_finalizing_still_publishes_the_older_claims() {
    let mut s = store_with_ledger(None);          // main ledger rows carry the local id "p1"
    s.register_project("p1", "C:/r", Some("owner/repo")).unwrap();
    assert_eq!(sync(&mut s, true).unwrap().len(), 2);
}
#[test] fn a_second_local_id_with_the_same_name_joins_the_same_chain() {
    let mut s = store_with_ledger(Some("owner/repo"));
    sync(&mut s, true).unwrap();
    let mut e = edit("z", 200_000); e.project = Some("p9".into());   // another worktree
    s.register_project("p9", "C:/w", Some("owner/repo")).unwrap();
    s.append_events(&[e]).unwrap();
    s.set_watermark("src", 2_000_000).unwrap();
    finalize(&mut s).unwrap();
    assert_eq!(sync(&mut s, true).unwrap().len(), 1);
    let entries: Vec<_> = s.published_rows("owner/repo").unwrap().into_iter().map(|r| r.entry).collect();
    assert!(tally_scoring::verify_chain(&entries, GENESIS_HASH).is_ok());
}
#[test] fn dry_run_writes_nothing() {
    let mut s = store_with_ledger(Some("owner/repo"));
    assert_eq!(sync(&mut s, false).unwrap().len(), 2);
    assert!(s.published_projects().unwrap().is_empty());
}
#[test] fn a_new_ruleset_starts_a_new_published_chain() {
    // append two main entries by hand with append_ledger: ruleset "r1" then "r2", both project "owner/repo",
    // linked in the main chain (rehash each with entry_hash). Register "owner/repo" for any local id.
    // sync: two published rows, each with prev_hash GENESIS_HASH; each (project, ruleset) chain verifies alone.
}
#[test] fn a_tampered_main_or_published_ledger_stops_everything() {
    // drop the triggers (as in the status.rs test), change a main row's verified_s:
    //   sync -> Err(MainLedger(..)) and published_rows is empty
    // separately: sync once, drop the published triggers, change a published row's entry_json verified_s:
    //   sync -> Err(PublishedChain { reason: "bad_hash", .. }) or Err(Inconsistent { .. }); published_rows unchanged
}
#[test] fn bad_public_names_are_rejected() {
    for bad in ["", "owner", "a/b/c", "../x", "o/..", "o/r r", "o/r?x"] { assert!(!valid_public_name(bad), "{bad}"); }
    assert!(valid_public_name("sectersion/tally"));
    // a registered bad name: sync returns Err(BadName(..)) and writes nothing
}
#[test] fn badge_hours_round_half_up_to_one_decimal() {
    assert_eq!(hours_message(0), "0.0 h");
    assert_eq!(hours_message(179), "0.0 h");
    assert_eq!(hours_message(180), "0.1 h");      // 0.05 h rounds up
    assert_eq!(hours_message(3600), "1.0 h");
    assert_eq!(hours_message(45_000), "12.5 h");
    assert_eq!(badge_json(660),
        "{\"color\":\"blue\",\"label\":\"tracked with Tally\",\"message\":\"0.2 h\",\"schemaVersion\":1}\n");
}
```

Fill in the three tests sketched as comments; they must assert exactly as described.

- [ ] **Step 2:** Run `cargo test -p tally-daemon published::`. Expected: FAIL (module missing).
- [ ] **Step 3:** Implement to the interfaces.
- [ ] **Step 4:** Run `cargo test -p tally-daemon`. Expected: PASS, no warnings.
- [ ] **Step 5:** Commit: `feat(daemon): derive per-project published chains and badge JSON`.

---

### Task 4: verify-published

**Files:**
- Modify: `crates/tally-daemon/src/published.rs`, `crates/tally-daemon/src/main.rs`, `crates/tally-daemon/tests/cli.rs`

**Interfaces:**
- Produces (`published.rs`): `pub struct PublishedCheck { pub entries: usize, pub project: Option<String>, pub ruleset: Option<String>, pub verified_s: u64 }` and `pub fn verify_published_json(json: &str) -> Result<PublishedCheck, String>`. Parses a JSON array of `LedgerEntry`. Errors: the serde message for bad JSON; `FAIL at entry {index}: {reason}` for a chain failure (`verify_chain(.., GENESIS_HASH)`); `FAIL: file mixes projects or rulesets` when the entries are not all one project and one ruleset. An empty array is valid (`entries: 0`).
- CLI: `tally verify-published <FILE>...`. It needs no home and no database: handle it in `main.rs` before the `home`/`db` logic, so the store is opened only for the other commands. For each file prints `ok: <file>: <n> entries, project <name>, ruleset <r>, verified_hours <N.N>` (hours as in `hours_message` without the ` h`) or the `FAIL` text; after all files it exits 1 if any failed. A missing file is an error naming it.

- [ ] **Step 1: Write the failing tests.** Unit (in `published.rs`): build a 2-entry chain with `tally_scoring::chain` from claims with `project` `"o/r"`, serialize it as a JSON array: ok, `entries == 2`, `verified_s` matches the sum. Change one `verified_s`: the error starts with `FAIL at entry`. Two projects mixed: the mixed-file error. Also assert `tally_scoring::wasm::verify_chain_json_impl(&json, GENESIS_HASH) == Ok(-1)` for the good file if that function is reachable natively (it is `pub` in `wasm.rs`; if the module is feature-gated, skip this assertion and say so in the commit message). CLI (in `tests/cli.rs`): write a good file and run with a temp `--home`: exit 0, stdout contains `ok:` and `verified_hours`; tamper the file: exit 1, stdout contains `FAIL at entry`; after both runs `<home>/tally.db` does not exist.
- [ ] **Step 2:** Run `cargo test -p tally-daemon verify_published`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test -p tally-daemon`. Expected: PASS.
- [ ] **Step 5:** Commit: `feat(daemon): tally verify-published checks a downloaded chain without daemon state`.

---

### Task 5: Hosting on the tally-data branch

**Files:**
- Create: `crates/tally-daemon/src/hosting.rs`, `crates/tally-daemon/src/publish.rs`
- Modify: `crates/tally-daemon/src/lib.rs` (`pub mod hosting; pub mod publish;`)

**Interfaces:**
- Produces (`hosting.rs`):
  - `pub struct Hosting { pub dir: PathBuf, pub remote: String, pub branch: String }` and `Hosting::new(home: &Path, remote: &str, branch: &str)`: `dir` = `<home>/publish/<sha256-hex-of-remote, first 12 chars>`. The remote URL never appears in a file name.
  - `#[derive(thiserror::Error, Debug)] pub enum HostingError { GitMissing, Git { args: String, stderr: String }, Io(#[from] std::io::Error) }`. `Git.args` holds the subcommand only (for example `push`), never the remote URL, so an error line cannot leak credentials embedded in a URL.
  - Every git call: `std::process::Command::new("git").current_dir(dir)` with env `GIT_TERMINAL_PROMPT=0`, `GCM_INTERACTIVE=never`, and `-c user.name=tally -c user.email=tally@localhost -c commit.gpgsign=false`. A missing binary maps to `GitMissing`.
  - `Hosting::sync_down(&self) -> Result<(), HostingError>`: creates `dir` and runs `git init` if there is no `.git`; sets `origin` to `remote` (add or `set-url`); `git fetch origin <branch>`. If the remote has the branch (fetch succeeds), `git checkout -f -B <branch> origin/<branch>` then `git reset --hard origin/<branch>` and `git clean -fd` (the clone is a cache; unpushed local commits from a failed push are discarded and regenerated). If the remote has no such branch (fetch error containing `couldn't find remote ref`), `git checkout --orphan <branch>` (ignore "already on"), `git rm -rf --ignore-unmatch .`. Any other fetch failure is returned as `Git`.
  - `Hosting::read(&self, rel: &str) -> Result<Option<String>, HostingError>`: file contents under `dir`, `None` if absent.
  - `Hosting::commit_and_push(&self, files: &[(String, String)], message: &str) -> Result<bool, HostingError>`: writes each `(relative path, contents)` (creating directories), `git add -A`, and if `git status --porcelain` is empty returns `Ok(false)` without committing or pushing; otherwise `git commit -m message` and `git push origin <branch>` (never `--force`) and returns `Ok(true)`.
- Produces (`publish.rs`):
  - `pub struct PublishConfig { pub enabled: bool, pub remote: Option<String>, pub branch: String }`, default `enabled=false`, `remote=None`, `branch="tally-data"` (the config-file wiring is Task 6). The remote for a project is `remote` if set, else `https://github.com/<public name>.git`.
  - `#[derive(thiserror::Error, Debug)] pub enum PublishFail { Published(#[from] PublishError), Hosting(#[from] HostingError), RemoteDiverged { project: String, file: String }, Json(#[from] serde_json::Error) }`
  - `pub struct PublishReport { pub projects: Vec<ProjectReport>, pub pushed: bool }` and `pub struct ProjectReport { pub name: String, pub new_entries: usize, pub total_entries: usize, pub verified_s: u64, pub files: Vec<String> }`.
  - `pub fn render(store: &Store, name: &str, extra: &[PublishedRow]) -> Result<Vec<(String, String)>, PublishFail>`: for project `name`, the stored rows plus `extra` (so a dry run can render rows that are not stored) grouped by ruleset into `<name>/ledger-<ruleset>.json` (`[` newline, one `serde_json::to_string(entry)` per line joined with `,` and newlines, newline `]` newline; an empty chain writes no file) and `<name>/badge.json` = `badge_json(sum of verified_s over all rows)`.
  - `pub fn publish_once(store: &mut Store, cfg: &PublishConfig, home: &Path, dry_run: bool) -> Result<PublishReport, PublishFail>`: (1) `published::sync(store, !dry_run)` (verifies both ledgers first). (2) Per published project (stored plus the dry-run rows): `render`. (3) If `dry_run`, return the report with no git and no network. (4) Otherwise group projects by remote, and for each: `sync_down`; for every ledger file, read the remote copy: it must parse as a list of entries that is a prefix of the local chain compared by `hash`; if it is longer than the local chain or any hash differs, return `RemoteDiverged` before writing anything; then `commit_and_push(files, "tally: publish <N> new entries")` where the message contains only the count. `pushed` is true if any commit was pushed.

- [ ] **Step 1: Write the failing tests** in `publish.rs`. Helpers: `bare_remote()` creates a temp dir and runs `git init --bare -b tally-data` in it, returning `(tempdir, url)` where `url` is the path string; `skip_without_git() -> bool` (true and prints a note if `git --version` fails; each test starts with `if skip_without_git() { return; }`). `cfg(url)` = `PublishConfig { enabled: true, remote: Some(url), branch: "tally-data".into() }`. `named_store()` = the Task 3 helper `store_with_ledger(Some("owner/repo"))` (make it `pub(crate)` in `published.rs` under `#[cfg(test)]`).

```rust
#[test] fn first_publish_creates_the_branch_with_badge_and_a_verifiable_chain() {
    // publish_once -> pushed == true, report.projects[0].total_entries == 2
    // clone the bare remote into a fresh tempdir (git clone -b tally-data), read owner/repo/ledger-2026.11-0.json
    // verify_published_json(text) is Ok with entries == 2 and project "owner/repo"
    // owner/repo/badge.json == badge_json(220)
    // the clone contains no other top-level entries than "owner" (and .git)
    // `git log --format=%an%n%s` shows author "tally" and a subject with no project id or path
}
#[test] fn second_publish_with_nothing_new_makes_no_commit() {
    // publish_once twice; the second returns pushed == false; `git rev-list --count tally-data` on the remote is 1
}
#[test] fn new_claims_extend_the_branch_by_one_commit() {
    // after the first publish, add an event and finalize, publish_once: pushed == true, commit count 2,
    // the remote ledger file is the first file plus one line; the earlier lines are byte-identical
}
#[test] fn a_failed_push_leaves_nothing_half_published_and_the_next_run_recovers() {
    // point cfg.remote at a path that does not exist: publish_once -> Err(Hosting(Git { .. })); published_rows is stored
    // then point at a real bare remote: publish_once succeeds and the remote has exactly the same files as a never-failed run
    // second variant: make the push fail after the commit (install a `pre-receive` hook in the bare repo that exits 1,
    // unix only; on Windows skip this variant), assert Err, remove the hook, publish_once succeeds, one commit on the remote
}
#[test] fn a_remote_chain_from_another_machine_is_refused() {
    // machine A publishes to the bare remote. Machine B is a different Store (different events, same public name,
    // different home): publish_once against the same remote -> Err(RemoteDiverged { .. }); remote head unchanged
}
#[test] fn a_ledger_that_does_not_verify_publishes_nothing() {
    // drop the main-ledger triggers, change a row, publish_once -> Err(Published(MainLedger(..))); bare remote has no branch
}
#[test] fn dry_run_touches_neither_the_store_nor_git() {
    // publish_once(dry_run = true) with cfg.remote pointing at a nonexistent path: Ok; report lists owner/repo with 2 new entries;
    // published_projects is empty; <home>/publish does not exist
}
#[test] fn git_errors_do_not_echo_the_remote_url() {
    // remote "https://user:secret@example.invalid/x.git": publish_once -> Err whose Display does not contain "secret"
    // (offline-safe: the failure may be a DNS error; the assertion is only about the message)
}
```

- [ ] **Step 2:** Run `cargo test -p tally-daemon publish::`. Expected: FAIL (modules missing).
- [ ] **Step 3:** Implement. Mind these pitfalls: `git init` must name the branch with `-b <branch>` (or `git checkout --orphan`) because `init.defaultBranch` differs per machine; compare remote and local chains by parsed `hash` fields, not by text; run `git status --porcelain` with `-uall`.
- [ ] **Step 4:** Run `cargo test -p tally-daemon`. Expected: PASS, no warnings.
- [ ] **Step 5:** Commit: `feat(daemon): publish per-project chains and badge to the tally-data branch`.

---

### Task 6: `tally publish [--dry-run]` and config

**Files:**
- Modify: `crates/tally-daemon/src/config.rs` (plan 03), `crates/tally-daemon/src/main.rs`, `crates/tally-daemon/tests/cli.rs`

**Interfaces:**
- Config: a `[publish]` table in `<home>/config.toml` with keys `enabled` (bool, default false), `remote` (string, optional), `branch` (string, default `tally-data`). Unknown keys are an error (rule C5). `Config` gets a field `publish: PublishConfig` (derive `Deserialize` with `deny_unknown_fields` on `PublishConfig`; if plan 03's loader uses another mechanism, follow it). There is no key for a token or password, by design.
- CLI: `tally publish [--dry-run]`. Loads the config, opens the store, calls `publish_once(.., dry_run)`. Output, one block per project:

```
project owner/repo: 2 new entries, 2 total, 0.1 h
  would write owner/repo/ledger-2026.11-0.json
  would write owner/repo/badge.json
```

with `would write` for `--dry-run` and `wrote` otherwise, followed by `pushed` or `nothing to push` (not printed for `--dry-run`, which ends with `dry run: nothing written, nothing pushed`). With no named project: `no named projects; nothing to publish`. A `PublishFail` exits non-zero with its message on stderr. Also prints the badge JSON for each project in dry-run mode so the maintainer sees the exact bytes.

- [ ] **Step 1: Write the failing tests** in `tests/cli.rs`: build a home with the lunch-scenario events (as in `lunch_scenario_end_to_end`), `project add <tempdir with .git> --name owner/repo` and give the events that project id (or ingest events whose `project` is the id printed by `project add`), `finalize`; then `publish --dry-run`: exit 0, stdout contains `project owner/repo: 2 new entries` and `"message":"0.2 h"` and `dry run`; `tally status` afterwards unchanged; `publish_ledger` rows not created (assert by running `publish --dry-run` twice with identical output). A config file with `[publish]\nbogus = 1\n` makes `publish` exit non-zero naming `bogus`. A real publish against a local bare remote (config `remote = "<path>"`) prints `pushed`, and a second run prints `nothing to push`. Skip the bare-remote test when git is missing.
- [ ] **Step 2:** Run `cargo test -p tally-daemon --test cli publish`. Expected: FAIL.
- [ ] **Step 3:** Implement. `main.rs` stays argument parsing and printing; the report formatting lives in a small `PublishReport::render_text(dry_run: bool) -> String` in `publish.rs`.
- [ ] **Step 4:** Run `cargo test -p tally-daemon` and `cargo clippy -p tally-daemon --all-targets`. Expected: PASS, no warnings.
- [ ] **Step 5:** Commit: `feat(daemon): tally publish command and [publish] config`.

---

### Task 7: Daily scheduling inside `tally run`

**Files:**
- Modify: `crates/tally-daemon/src/publish.rs`, `crates/tally-daemon/src/run.rs` (plan 03), `crates/tally-daemon/Cargo.toml` (`chrono = { version = "0.4", default-features = false, features = ["clock"] }`)

**Interfaces:**
- Produces (`publish.rs`):
  - `pub const RETRY_MS: i64 = 3_600_000;`
  - `pub fn local_day_number(now: chrono::DateTime<chrono::Local>) -> i64`: `year * 10000 + month * 100 + day` (for example `20261123`). The caller passes `chrono::Local::now()`; tests pass fixed values.
  - `pub struct Scheduler { last_failure_ms: Option<i64> }` with `Scheduler::new()` and `pub fn due(&self, store: &Store, today: i64, now_ms: i64, enabled: bool) -> Result<bool, StoreError>`: false if `!enabled`; false if `meta_get("last_publish_day") == Some(d)` with `d >= today`; false if `last_failure_ms` is within `RETRY_MS` of `now_ms`; else true. So a day missed while the daemon was off (`d < today`) is caught up at next start, once, not once per missed day.
  - `pub fn tick(&mut self, store: &mut Store, cfg: &PublishConfig, home: &Path, today: i64, now_ms: i64)`: if `due`, call `publish_once(.., false)`. On `Ok`: `meta_set("last_publish_day", today)`, clear `last_failure_ms`, print one line to stderr only if something was pushed (`published N entries`). On `Err`: set `last_failure_ms = Some(now_ms)` and print `publish failed: <error>` to stderr. It never returns an error and never panics on a publish failure.
- `run.rs`: create one `Scheduler` before the loop and call `scheduler.tick(&mut store, &config.publish, &home, local_day_number(chrono::Local::now()), now_ms)` once per iteration, after `finalize`. The call is cheap when not due (one meta read).

- [ ] **Step 1: Write the failing tests** in `publish.rs` (fixed `today`, `now_ms`; named store and a bare remote from Task 5):

```rust
#[test] fn runs_once_per_day_and_again_the_next_day() {
    // tick(today=20261123): publishes (pushed). tick again same day: due == false, no git call
    //   (point cfg.remote at a nonexistent path after the first tick: the second tick must not error or set last_failure_ms)
    // tick(today=20261124): due, runs, no new entries -> no commit; meta last_publish_day == 20261124
}
#[test] fn a_missed_day_is_caught_up_once_at_next_start() {
    // meta last_publish_day = 20261120, new Scheduler, tick(today=20261125): publishes once; meta == 20261125; next tick is not due
}
#[test] fn a_day_with_no_new_claims_is_marked_done_without_a_commit() {
    // first tick publishes; add no events; tick next day: report.pushed false (remote commit count unchanged), meta advanced
}
#[test] fn a_failure_is_retried_after_an_hour_not_every_tick() {
    // cfg.remote = nonexistent path. tick(now=0): fails, meta unset. tick(now=RETRY_MS - 1): not due.
    // fix the remote; tick(now=RETRY_MS): runs and succeeds; meta set
}
#[test] fn disabled_never_publishes() {
    // enabled=false: due() is false even with an unpublished ledger
}
#[test] fn local_day_number_formats_year_month_day() {
    // a fixed chrono Local date 2026-11-23 -> 20261123 (construct with Local.with_ymd_and_hms(..).unwrap())
}
```

- [ ] **Step 2:** Run `cargo test -p tally-daemon scheduler runs_once`. Expected: FAIL.
- [ ] **Step 3:** Implement `Scheduler` and wire `run.rs`. If `run.rs` has a test with a fake clock, add one assertion there that a loop iteration with `publish.enabled = false` performs no git call (for example `cfg.remote` pointing at a nonexistent path and the loop still returns Ok).
- [ ] **Step 4:** Run `cargo test -p tally-daemon`. Expected: PASS, no warnings.
- [ ] **Step 5:** Commit: `feat(daemon): daily publish inside tally run with catch-up and retry`.

---

### Task 8: README badge, verification note, and a live check

**Files:**
- Modify: `README.md`

**Interfaces:**
- The badge line goes directly under the `# Tally` title (a badge that is not live yet shows "invalid" until the first publish; add it in the same merge as the first publish, see Step 4):

```markdown
[![tracked with Tally](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Fsectersion%2Ftally%2Ftally-data%2Fsectersion%2Ftally%2Fbadge.json)](https://github.com/sectersion/tally/tree/tally-data/sectersion/tally)
```

- A new section `## How this badge is made` after "Why keep it installed", containing, in this order: (a) one sentence: the number is verified hours only, all-time for this repository, counted from the day the daemon first ran on the maintainer's machine (the start date is filled in at first publish); earlier work and time while the daemon was off are not counted; (b) the files on the `tally-data` branch (`sectersion/tally/badge.json` and `sectersion/tally/ledger-<ruleset>.json`) and that they update about once a day, with the badge lagging up to about 15 minutes behind a publish because of caching; (c) the exact verification commands:

```
curl -LO https://raw.githubusercontent.com/sectersion/tally/tally-data/sectersion/tally/ledger-2026.11-0.json
tally verify-published ledger-2026.11-0.json
```

with the expected output shape (`ok: ... entries, project sectersion/tally, ruleset 2026.11-0, verified_hours N.N`) and the statement that the check needs no daemon or account; (d) what the check proves and does not: the chain is intact and the hours add up, but a ledger holds claims, not events, so it cannot be re-scored; (e) that each entry's timestamp is public and permanent in the branch history; (f) that a new ruleset starts a new file and the badge sums all files.

- [ ] **Step 1:** Add the section. Do not add the badge line yet.
- [ ] **Step 2:** Live check on the maintainer's machine, with `git push` auth working (`gh auth status`, `gh auth setup-git`): `tally project add . --name sectersion/tally`, let the daemon finalize some claims, then `tally publish --dry-run` and read the output, then `tally publish`. Confirm on GitHub that branch `tally-data` exists with only `sectersion/tally/*` files, that `curl` of the raw `badge.json` returns the expected JSON, and that `tally verify-published` on the downloaded ledger prints `ok`. Revoke test: run `gh auth logout`, run `tally publish`, and confirm it fails with a git error and exits non-zero without touching local state; log back in and confirm it recovers.
- [ ] **Step 3:** Set `[publish] enabled = true` in `<home>/config.toml`, restart `tally run`, and confirm one publish per day (check `git log tally-data` after the first midnight crossing).
- [ ] **Step 4:** Add the badge line to the README and fill in the start date in the note. Commit: `docs: README badge and how it is made`. Push to `main` only after Step 2 succeeded.

---

## Done when

- `cargo test -p tally-daemon` passes on all three operating systems in CI, including the bare-remote publish tests.
- `tally publish --dry-run` prints exactly what would leave the machine and writes nothing; `tally publish` pushes one commit to `tally-data` containing only `sectersion/tally/*`, and a second run with no new claims makes no commit.
- A chain file downloaded from the branch passes `tally verify-published` with no daemon state, and the same file passes the WASM `verifyChainJson`.
- A failed push, a divergent remote (second machine), a tampered ledger, and a ruleset change each stop the publish with a clear message and leave the main ledger untouched.
- The README shows the live badge and the verification command.

## Not in this plan

- Starting a new main-ledger chain after a ruleset bump (re-scoring history), see Decision 6.
- Publishing events, or anything but final claims; re-scoring from published data.
- Multiple publishers per project, merging chains from several machines, renaming a published project.
- Gist, Pages, or server-hosted badges (the server's `GET /v1/projects/{id}/badge.json`, contract C7, replaces this later in plan 09; the per-project chains from this plan are what plan 07 uploads).
- Systemd or Windows service registration for the daemon (plan 12).
