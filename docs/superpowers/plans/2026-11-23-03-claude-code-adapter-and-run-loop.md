# Claude Code Adapter and Run Loop Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `tally run` polls event sources on a timer, stores their events, moves watermarks, persists per-source cursors and finalizes claims. The first source reads Claude Code session logs (`~/.claude/projects/<dir>/*.jsonl`) into `prompt`, `interrupt`, `tool_decision`, `agent_output`, `agent_busy`, and agent-initiated `test_run`/`commit` events, with `relevance_pm` and `novelty_bytes` computed locally.

**Architecture:** A `Source` trait (contract C1) in `source.rs` and a runner with per-tick hooks in `run.rs`; a `source_state` table in the store; a `config.toml` loader; and a `claude_code` module split into a pure line classifier, a complete-line file tailer (byte cursors in source state), and a per-session state machine that emits events. Text is read, turned into numbers, and dropped in memory.

**Tech Stack:** existing crate dependencies, plus `tally-scoring` feature `text`, `serde = { version = "1", features = ["derive"] }`, `toml = "0.8"`, `ctrlc = "3"`.

**Spec:** `docs/superpowers/plans/README.md` (contracts C1 to C5), `docs/superpowers/specs/2026-11-22-daemon-roadmap-design.md` (section 2, "Fixtures"), `docs/spec/scoring.md` (§2 events, §5 relevance and novelty). Read all three before every task.

## Global Constraints

- Reuse, do not copy: `tally_scoring::{Event, Kind, Decision, Actor}`, `tally_scoring::text::{Vocab, prompt_features}`, `tally_scoring::ruleset::CONTEXT_TAIL_BYTES`, `tally_daemon::project::{find_git_root, normalize_path, project_id_from_normalized}`, `Store`, `finalize`.
- No text on disk (contract C3): prompt text, agent output, tool output, commands and file contents are never written to the store, to source state, to stderr, or to test output. Source state holds only byte offsets, timestamps, ids and hashes. `cmd_hash` = lowercase hex SHA-256 of the command string.
- Event ids are stable derived strings (contract C2): `claude:<sessionId>:<entry uuid>:<kind>`, plus `:<n>` when one entry yields several events of one kind. Re-reading a file must produce identical events.
- The event `source` field is `claude-code` (spec §2). `Source::id()` (watermark and state name) is also `claude-code`.
- Watermark contract (C1): a returned watermark is below the start of every open `agent_busy` span and below the timestamp of every line not yet read in full.
- One source's error never stops the runner or other sources: it is printed to stderr (no log text in the message) and the source is polled again next tick.
- Sidechains (`isSidechain: true`, and every file under `<session>/subagents/`) and task notifications never produce human-action events.
- All time values are `i64` milliseconds. Log timestamps are RFC 3339 UTC strings; parse them without a new dependency (Task 7).
- Tests never read the real `~/.claude`; they use committed fixtures and tempdirs. `cargo test -p tally-daemon` passes on Windows, macOS and Linux.

## Review Focus

1. **A log line is being appended while it is read.** Expected: a final line without `\n` is not parsed, the cursor stays at its start, it is read in full on the next poll, nothing is lost or duplicated. Test in Task 8.
2. **A prompt (or any line) of any size,** far larger than the read buffer and the 8 KiB context tail. Expected: correct `chars` (Unicode scalar count, saturating at `u32::MAX`), no panic, at most one line held in memory per file. Tests in Tasks 8 and 9.
3. **Non-UTF-8 bytes or malformed JSON in a line.** Expected: the line is skipped and counted, the cursor moves past it, the rest of the file is read; never a panic or a stuck cursor. Tests in Tasks 7 and 8.
4. **The watermark never passes an open span.** Expected: while a turn runs (human prompt seen, no `turn_duration` yet) the watermark stays below the turn's start; once closed, the `agent_busy` span is emitted and the watermark moves on. Test in Task 10.
5. **Re-reading, rotation, concurrent sessions.** Expected: re-reading every file from offset 0 adds zero events (all duplicates); a file that shrinks is re-read from 0 without duplicates; two sessions written at once keep separate cursors and spans. Tests in Tasks 8 and 11.
6. **Human vs agent.** Expected: only `origin.kind == "human"` prompts, user interrupts, and permission decisions whose `source` is not `config` are human actions; task notifications, tool results, subagent files and auto-approved tools are not. Tests in Tasks 7 and 11.

## File Structure

```
docs/spec/scoring.md                                 §2 `id` row amended (Task 1)
crates/tally-daemon/
  Cargo.toml                                         + tally-scoring "text", serde, toml, ctrlc
  src/store.rs                                       source_state table, get_state/set_state, WAL
  src/source.rs                                      Source, Poll, SourceState, SourceError, now_ms
  src/run.rs                                         TickHook, HookError, run_once, run_loop, sources, hooks
  src/config.rs                                      Config from <home>/config.toml, unknown keys rejected
  src/project.rs                                     + project_for_dir (Task 11)
  src/claude_code/mod.rs                             ClaudeCode: Source impl, file discovery, wiring
  src/claude_code/line.rs                            pure classifier: one JSON line -> Entry
  src/claude_code/tail.rs                            complete-line reader with a byte cursor
  src/claude_code/session.rs                         per-session state machine -> events, open span
  src/claude_code/features.rs                        project vocabulary (git ls-files), context tail
  src/main.rs                                        `tally run [--interval-secs N] [--once]`
  tests/fixtures/claude_code/FORMAT.md               structure notes from the spike (no text)
  tests/fixtures/claude_code/projects/<dir>/<session>.jsonl                  scrubbed sessions
  tests/fixtures/claude_code/projects/<dir>/<session>/subagents/agent-x.jsonl
  tests/claude_code.rs                               fixture end-to-end through `tally run --once`
```

## Assumes from earlier plans (exact names; code on `main` wins)

- `tally_daemon::store::{Store, StoreError, AppendSummary}`: `Store::open`, `Store::open_in_memory`, `append_events`, `set_watermark`, `horizon`, `finalized_through`, `register_project`, `project_names`, `events`, `ledger`; field `pub(crate) conn`; `Store::open` already sets `busy_timeout(5 s)`.
- `tally_daemon::finalize::{finalize, FinalizeSummary, FinalizeError}`. **Differs from the index:** `Store::append_ledger` takes a third argument `scored_events: usize`, `StoreError::Conflict` exists, and `finalize` retries internally up to 3 times. The runner only calls `finalize`.
- `tally_daemon::project::{find_git_root, normalize_path, project_id_from_normalized, project_id, register_git_project}` (`register_git_project` lives in `project.rs`, not `main.rs`). There is no "project for a directory" lookup yet; Task 11 adds one.
- `tally_daemon::ingest::ingest_ndjson` (unchanged).
- `crate::test_support::{ev, prompt, edit}` (`#[cfg(test)]`).
- `tally_scoring::text` behind feature `text`: `Vocab::from_texts`, `Vocab::extend` (stops at `VOCAB_MAX`), `prompt_features(prompt, &vocab, context_tail) -> (u16, u32)`. The daemon does not enable `text` yet.
- CLI: global `--home` / `TALLY_HOME`, database `<home>/tally.db`; `main.rs` creates `<home>` only for `ingest` and `project` (Task 5 adds `run`).

## Decisions I made (maintainer to confirm)

1. **Id format** `claude:<sessionId>:<uuid>:<kind>[:<n>]`. C2's example was `claude:<native-id>:<kind>`; the entry `uuid` is the native id, and the session id is added so ids stay unique if a uuid repeats across sessions (resumed or forked sessions copy history).
2. **Live capture only:** on first sight of a file with no cursor, the source starts at its current end. A file first seen after the daemon started (a new session) is read from 0. Backfill is out of scope (README ruling).
3. **Human tool decision** = a `permissionDecision` whose `source` is not `config` (observed: `config` for mode, rule and classifier auto-decisions; `user_temporary` for a user click). `accept` maps to `approve`, `reject` to `deny`. Auto decisions produce nothing. The spike lists every observed `source` value.
4. **Interrupt** = a non-sidechain `user` entry with no `toolUseResult` whose first text block starts with `[Request interrupted by user` (observed shape: text content, no `origin`). Confirmed or corrected by the spike.
5. **Agent busy span** = human prompt `ts` to the `ts` of the turn's `system`/`turn_duration` entry. A turn also closes at an interrupt or at the next human prompt (at the last seen entry). A turn without `turn_duration` for `OPEN_SPAN_MAX_MS` (6 h) is closed at its last seen entry, so a crashed session cannot hold the horizon forever.
6. **Agent test/commit detection** from Bash `tool_use.input.command`, using a fixed prefix list (Task 10). The log has no commit sha, so `sha = "claude:" + <tool_use id>` and `lines_changed = 0`; plan 04's git source records real commits and supersedes this. Plan 04 matches this event by `by = agent` plus a time window (the sha does not matter) and gates its own commits on this source's watermark (README ruling 1), so the watermark must stay below any pending Bash `git commit` tool_use (Decision 9, tested in Task 10).
7. **Projects:** an event's `project` is the local id of `find_git_root(cwd)`, `None` outside a repo. Nothing is auto-registered; `finalize` already leaves unnamed ids as they are.
8. **Config keys** (all optional): `[run] interval_secs = 15`; `[claude_code] enabled = true`, `projects_dir = "<user home>/.claude/projects"`.
9. **Watermark** = `min(now - WRITE_SLACK_MS, open turn starts - 1, pending Bash tool_use ts - 1)` with `WRITE_SLACK_MS = 120 s`, so entries whose timestamp is earlier than their flush, and a partial last line, cannot fall under it. Claims therefore finalize at least about 4 minutes after they happen.
10. **No text across restarts:** context tail, last output and last error are memory only. After a restart the source re-reads up to 64 KiB before each cursor without emitting, to rebuild them; the first prompt after a restart can still score slightly differently from a one-shot read, which only matters if ids collide (they do not: same ids, first one wins).

## Source authoring notes (put this text in the `source.rs` module doc)

- A source that first appears with a watermark below `finalized_through` has promised nothing about the past. Any event it emits at or before `finalized_through` is rejected by `append_events` with `LateEvent`. The runner drops only the late events (and retries the rest), still persists the source's state, so the source does not resend it forever, and prints `source <id>: dropped <n> late events`.
- Sources never call `finalize` and never touch the store; they read and write cursors only through `SourceState`. Extra per-source data (coverage, health, agent writes) goes through default trait methods that the runner in `run.rs` persists (README ruling 5).
- Event ids must be derived from the log, never random, so a re-read is idempotent.

---

### Task 1: Spec amendment for stable event ids (contract C2)

**Files:**
- Modify: `docs/spec/scoring.md` (§2 table, `id` row; §9 Decisions, new item 4)

- [ ] **Step 1:** In §2, change the `id` row's Notes from `ULID` to: `Unique string. Adapters derive it from the source log (for example claude:<session>:<entry>:prompt) so re-reading a log yields the same ids.`
- [ ] **Step 2:** Append to §9: `4. **Event ids:** stable derived strings, not ULIDs, so a re-read log is deduplicated by id. Nothing orders by id except as the tie-break in (ts, id).`
- [ ] **Step 3:** Run `cargo test -p tally-scoring`. Expected: PASS (no code depends on ULIDs; the ruleset version is unchanged because no constant or rule changed).
- [ ] **Step 4:** Commit: `docs(spec): event ids are stable derived strings`.

---

### Task 2: Source state table and WAL

**Files:**
- Modify: `crates/tally-daemon/src/store.rs`

**Interfaces:**
- Produces:
  - Schema: `CREATE TABLE IF NOT EXISTS source_state(source TEXT NOT NULL, key TEXT NOT NULL, value TEXT NOT NULL, PRIMARY KEY(source, key));`
  - `pub fn get_state(&self, source: &str) -> Result<BTreeMap<String, String>, StoreError>`: all keys for one source.
  - `pub fn set_state(&mut self, source: &str, values: &BTreeMap<String, String>) -> Result<(), StoreError>`: one transaction; upserts every pair (keys not in `values` are kept).
  - `Store::open` runs `PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;` after `busy_timeout`. `open_in_memory` is unchanged.

Whole-map get/set (rather than one key per call) lets the runner load a source's state once per poll and write it back in one transaction.

- [ ] **Step 1: Write the failing tests** in `store.rs`:

```rust
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
#[test] fn file_store_uses_wal() {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::open(&dir.path().join("tally.db")).unwrap();
    let mode: String = s.conn.query_row("PRAGMA journal_mode", [], |r| r.get(0)).unwrap();
    assert_eq!(mode, "wal");
}
```

- [ ] **Step 2:** Run `cargo test -p tally-daemon store`. Expected: FAIL (methods not defined).
- [ ] **Step 3:** Add the table to `SCHEMA`, the two methods, and the pragmas in `open` (`journal_mode` returns a row: use `query_row`, not `execute`).
- [ ] **Step 4:** Run `cargo test -p tally-daemon`. Expected: all PASS, including the existing two-handle tests under WAL.
- [ ] **Step 5:** Commit: `feat(daemon): source state table and WAL journal`.

---

### Task 3: Source contract and runner

**Files:**
- Create: `crates/tally-daemon/src/source.rs`, `crates/tally-daemon/src/run.rs`
- Modify: `crates/tally-daemon/src/lib.rs` (`pub mod source; pub mod run;`)

**Interfaces:**
- Consumes: `Store::{append_events, set_watermark, get_state, set_state}`, `finalize`.
- Produces (`source.rs`), contract C1 exactly. Later plans add **default** methods to `Source` that the runner persists after `poll` (README ruling 5): `take_coverage` (plan 04), `take_health` and `take_agent_writes` (plan 08). This plan defines none of them; `run_once` step 3 is where they slot in.

```rust
pub struct Poll { pub events: Vec<tally_scoring::Event>, pub watermark: Option<i64> }
#[derive(thiserror::Error, Debug)]
pub enum SourceError { #[error("io: {0}")] Io(#[from] std::io::Error), #[error("{0}")] Other(String) }
pub trait SourceState { fn get(&self, key: &str) -> Option<String>; fn set(&mut self, key: &str, value: &str); }
impl SourceState for std::collections::BTreeMap<String, String> { /* get = map get, set = insert */ }
pub trait Source {
    fn id(&self) -> &str;
    fn poll(&mut self, now_ms: i64, state: &mut dyn SourceState) -> Result<Poll, SourceError>;
}
pub fn now_ms() -> i64; // SystemTime::now() since UNIX_EPOCH, in ms
```

- Produces (`run.rs`, README ruling 6; plans 04, 05, 07, 08 and 12 use these names):

```rust
#[derive(thiserror::Error, Debug)] #[error("{0}")] pub struct HookError(pub String);
/// Work done once per tick after `finalize` (plan 05: daily publish, plan 07: sync). Gets the store and `now_ms`.
pub type TickHook = Box<dyn FnMut(&mut Store, i64) -> Result<(), HookError>>;
#[derive(Debug, Default)]
pub struct TickReport { pub added: usize, pub duplicates: usize, pub errors: Vec<String>, pub finalized: Option<FinalizeSummary> }
pub fn run_once(store: &mut Store, sources: &mut [Box<dyn Source>], hooks: &mut [TickHook], now_ms: i64) -> TickReport;
// Task 5 adds, once `Config` exists:
pub fn sources(cfg: &Config, home: &std::path::Path, store: &mut Store) -> Result<Vec<Box<dyn Source>>, StoreError>;
pub fn hooks(cfg: &Config, home: &std::path::Path) -> Vec<TickHook>;
pub fn run_loop(store: &mut Store, cfg: &Config, home: &std::path::Path, stop: &std::sync::atomic::AtomicBool);
```

`sources` takes the store (read only) because plan 04's git source is built from `store.repos_to_poll()` and opens its own connection to the database file under `home`. Plans 05 and 07 push their hooks in `hooks`.

`run_once`, per source in order (the part the signatures do not determine):
1. `let mut st = store.get_state(id)` (a `BTreeMap`; a read error is pushed to `errors` and the source is skipped); `poll(now_ms, &mut st)`.
2. `Err(e)`: push `"source <id>: <e>"` to `errors`; persist nothing (the cursor did not move, the next tick retries).
3. `Ok(p)`: `append_events(&p.events)`.
   - `Ok`: add the counts; `set_watermark(id, w)` if `Some(w)`; `set_state(id, &st)`.
   - `Err(StoreError::LateEvent { .. })`: push `"source <id>: dropped <n> late events"` (`n` = the number of late events dropped; the rest of the batch is kept); still set the watermark and `set_state` (see the authoring notes).
   - any other error: push it; persist nothing (the events are re-emitted next tick and deduplicated).
4. After all sources: `finalize(store)`; `Ok` goes in `finalized`, `Err` is pushed to `errors`.
5. Then every hook in order: `hook(store, now_ms)`; an `Err(e)` is pushed as `"hook: <e>"` and the next hook still runs.

Events are appended before state is persisted, so a crash in between only causes a re-read that the id dedup absorbs. `run_once` never returns early because of one source.

`run_loop` (Task 5): build `sources(cfg, home, store)` and `hooks(cfg, home)` once; until `stop` is set: `run_once(.., now_ms())`, print each error to stderr, then sleep `cfg.run.interval_secs` in slices of at most 200 ms, checking `stop` between slices. It returns after the tick in progress completes (graceful shutdown: no tick is cut in half).

- [ ] **Step 1: Write the failing tests** in `run.rs`. Helper `struct Fake { id: &'static str, script: VecDeque<Result<Poll, SourceError>>, polls: usize }` whose `poll` increments `polls`, writes `state.set("polls", &polls.to_string())`, and pops the front of `script`. Events come from `crate::test_support::prompt`.

```rust
#[test] fn a_failing_source_does_not_stop_the_others() {
    let mut s = Store::open_in_memory().unwrap();
    let mut srcs: Vec<Box<dyn Source>> = vec![
        Box::new(Fake::new("bad", vec![Err(SourceError::Other("boom".into()))])),
        Box::new(Fake::new("good", vec![Ok(Poll { events: vec![prompt("a", 0, 600)], watermark: Some(500_000) })])),
    ];
    let r = run_once(&mut s, &mut srcs, &mut [], 0);
    assert_eq!(r.added, 1);
    assert_eq!(r.errors, ["source bad: boom"]);
    assert!(s.get_state("bad").unwrap().is_empty());           // failed poll persists nothing
    assert_eq!(s.get_state("good").unwrap()["polls"], "1");
    assert_eq!(s.horizon().unwrap(), Some(500_000));
    assert_eq!(r.finalized.unwrap().final_total, 1);           // finalize ran
}
#[test] fn no_watermark_leaves_the_horizon_alone() {
    let mut s = Store::open_in_memory().unwrap();
    let mut srcs: Vec<Box<dyn Source>> = vec![Box::new(Fake::new("x", vec![Ok(Poll { events: vec![], watermark: None })]))];
    run_once(&mut s, &mut srcs, &mut [], 0);
    assert_eq!(s.horizon().unwrap(), None);
}
#[test] fn late_batch_is_dropped_but_state_moves_on() {
    let mut s = Store::open_in_memory().unwrap();
    s.append_events(&[prompt("old", 0, 600)]).unwrap();
    s.set_watermark("other", 1_000_000).unwrap();
    crate::finalize::finalize(&mut s).unwrap();                // finalized_through = 1_000_000
    let mut srcs: Vec<Box<dyn Source>> = vec![Box::new(Fake::new("new",
        vec![Ok(Poll { events: vec![prompt("late", 10, 600)], watermark: Some(2_000_000) })]))];
    let r = run_once(&mut s, &mut srcs, &mut [], 0);
    assert_eq!(r.errors, ["source new: dropped 1 late events"]);
    assert_eq!(s.get_state("new").unwrap()["polls"], "1");
    assert_eq!(s.events().unwrap().len(), 1);
}
#[test] fn a_failing_hook_does_not_stop_the_next() {
    let mut s = Store::open_in_memory().unwrap();
    let seen = std::rc::Rc::new(std::cell::RefCell::new(vec![]));
    let (a, b) = (seen.clone(), seen.clone());
    let mut hooks: Vec<TickHook> = vec![
        Box::new(move |_, now| { a.borrow_mut().push(now); Err(HookError("nope".into())) }),
        Box::new(move |_, now| { b.borrow_mut().push(now); Ok(()) }),
    ];
    let r = run_once(&mut s, &mut [], &mut hooks, 7);
    assert_eq!(r.errors, ["hook: nope"]);
    assert_eq!(*seen.borrow(), [7, 7]);
    assert!(r.finalized.is_some());
}
```

- [ ] **Step 2:** Run `cargo test -p tally-daemon run::`. Expected: FAIL (modules missing).
- [ ] **Step 3:** Implement to the interfaces (`sources`, `hooks`, `run_loop` wait for Task 5). Put the "Source authoring notes" section above in the `source.rs` module doc comment and the C1 watermark contract on the `Source` trait.
- [ ] **Step 4:** Run `cargo test -p tally-daemon`. Expected: PASS, no warnings.
- [ ] **Step 5:** Commit: `feat(daemon): source contract and run_once with tick hooks`.

---

### Task 4: Config file

**Files:**
- Create: `crates/tally-daemon/src/config.rs`
- Modify: `crates/tally-daemon/Cargo.toml` (add `serde = { version = "1", features = ["derive"] }`, `toml = "0.8"`), `src/lib.rs` (`pub mod config;`)

**Interfaces:**
- Produces:

```rust
#[derive(serde::Deserialize, Debug, Default, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct Config { pub run: RunConfig, pub claude_code: ClaudeCodeConfig }
#[derive(serde::Deserialize, Debug, PartialEq)] #[serde(deny_unknown_fields, default)]
pub struct RunConfig { pub interval_secs: u64, pub dashboard: bool, pub dashboard_port: u16 }    // default 15, false, 7878 (plan 11 reads the last two)
#[derive(serde::Deserialize, Debug, PartialEq)] #[serde(deny_unknown_fields, default)]
pub struct ClaudeCodeConfig { pub enabled: bool, pub projects_dir: Option<std::path::PathBuf> }  // default true, None
#[derive(thiserror::Error, Debug)]
pub enum ConfigError {
    #[error("reading {path}: {source}")] Io { path: String, source: std::io::Error },
    #[error("{path}: {source}")] Parse { path: String, source: toml::de::Error },
}
pub fn load(home: &std::path::Path) -> Result<Config, ConfigError>;  // <home>/config.toml; missing file = Config::default()
```

`projects_dir: None` means `dirs::home_dir()/.claude/projects`, resolved by the Claude Code source (Task 11), not here. Later plans add their own tables to `Config` (C5).

- [ ] **Step 1: Write the failing tests** in `config.rs`:

```rust
#[test] fn missing_file_is_defaults() {
    let d = tempfile::tempdir().unwrap();
    let c = load(d.path()).unwrap();
    assert_eq!((c.run.interval_secs, c.claude_code.enabled, c.claude_code.projects_dir), (15, true, None));
    assert_eq!((c.run.dashboard, c.run.dashboard_port), (false, 7878));
}
#[test] fn dashboard_settings_parse() {
    // config.toml "[run]\ndashboard = true\ndashboard_port = 8123\n" -> run.dashboard == true, run.dashboard_port == 8123,
    // interval_secs still 15 (no unknown-field error)
}
#[test] fn partial_tables_keep_other_defaults() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("config.toml"), "[claude_code]\nenabled = false\n").unwrap();
    let c = load(d.path()).unwrap();
    assert_eq!((c.run.interval_secs, c.claude_code.enabled), (15, false));
}
#[test] fn unknown_keys_and_tables_are_errors() {
    let d = tempfile::tempdir().unwrap();
    for bad in ["[run]\ninterval = 5\n", "[nope]\n", "top = 1\n"] {
        std::fs::write(d.path().join("config.toml"), bad).unwrap();
        let e = load(d.path()).unwrap_err().to_string();
        assert!(e.contains("config.toml") && e.contains("unknown"), "{bad}: {e}");
    }
}
```

- [ ] **Step 2:** Run `cargo test -p tally-daemon config`. Expected: FAIL.
- [ ] **Step 3:** Implement; write manual `Default` impls for `RunConfig` (15) and `ClaudeCodeConfig` (`enabled: true`).
- [ ] **Step 4:** Run `cargo test -p tally-daemon`. Expected: PASS.
- [ ] **Step 5:** Commit: `feat(daemon): config.toml loader`.

---

### Task 5: `tally run`

**Files:**
- Modify: `crates/tally-daemon/src/main.rs`, `crates/tally-daemon/src/run.rs`, `crates/tally-daemon/Cargo.toml` (add `ctrlc = "3"`), `crates/tally-daemon/tests/cli.rs`

**Interfaces:**
- Produces: subcommand `tally run [--interval-secs N] [--once]`. `--interval-secs` overrides `config.run.interval_secs` (`main.rs` writes it into `cfg` before calling the library). `run` creates `<home>` the way `ingest` does. The source and hook lists are built in the library (`run.rs`, Task 3 signatures) so the binary stays thin: `sources(cfg, home, store)` returns an empty list until Task 11; `hooks(cfg, home)` returns an empty list (plans 05 and 07 add theirs). `run_loop(store, cfg, home, stop)` as described in Task 3.
- `--once`: one `run_once(&mut store, &mut sources(&cfg, &home, &store), &mut hooks(&cfg, &home), now_ms())`; prints `added <a>, duplicates <d>, final <total>, provisional <p>` (`0` for the last two if finalize failed); prints each error to stderr; exits 0 even when a source failed (a source failure is not a run failure); non-zero only when the config or store cannot be opened.
- Without `--once`: `ctrlc::set_handler` sets a `static STOP: AtomicBool`; `run_loop(&mut store, &cfg, &home, &STOP)`; on return prints `stopped`.

- [ ] **Step 1: Write the failing tests** in `tests/cli.rs`:

```rust
#[test] fn run_once_with_no_sources_finalizes_and_exits() {
    // fresh tempdir home <d>; <d>/config.toml = "[claude_code]\nprojects_dir = '<empty tempdir>'\n"
    //   (TOML literal string: Windows backslashes are safe; keeps the test off the real ~/.claude once Task 11 lands)
    // `tally --home <d> run --once`: exit 0, stdout "added 0, duplicates 0, final 0, provisional 0"
    // <d>/tally.db exists afterwards
}
#[test] fn run_refuses_a_bad_config() {
    // <d>/config.toml = "[run]\nbogus = 1\n": `run --once` exits non-zero, stderr contains "unknown field"
}
```

Unit test in `run.rs`:

```rust
#[test] fn run_loop_returns_when_stopped() {
    let stop = std::sync::atomic::AtomicBool::new(true);
    let d = tempfile::tempdir().unwrap();
    let mut s = Store::open_in_memory().unwrap();
    let mut cfg = Config::default();
    cfg.run.interval_secs = 3600; cfg.claude_code.enabled = false;
    run_loop(&mut s, &cfg, d.path(), &stop);  // must not hang
}
```

- [ ] **Step 2:** Run `cargo test -p tally-daemon --test cli` and `cargo test -p tally-daemon run::`. Expected: FAIL (no `run` subcommand, no `run_loop`).
- [ ] **Step 3:** Implement. `main.rs` only parses arguments, loads config, opens the store and calls the library.
- [ ] **Step 4:** Run `cargo test -p tally-daemon` and `cargo clippy -p tally-daemon --all-targets`. Expected: PASS, no warnings.
- [ ] **Step 5:** Commit: `feat(daemon): tally run`.

---

### Task 6: SPIKE: Claude Code session-log structure and a scrubbed fixture

The log format is not documented in this repo. This task inspects real logs for **structure only** and commits a synthetic fixture with the same shape. No product code.

**Files:**
- Create: `crates/tally-daemon/tests/fixtures/claude_code/FORMAT.md`, `crates/tally-daemon/tests/fixtures/claude_code/projects/__PROJECT__/aaaaaaaa-0000-4000-8000-000000000001.jsonl`, `.../__PROJECT__/aaaaaaaa-0000-4000-8000-000000000002.jsonl`, `.../__PROJECT__/aaaaaaaa-0000-4000-8000-000000000001/subagents/agent-a000000000000001.jsonl`

**Rules (privacy, non-negotiable):**
- Read logs under `C:/Users/Oliver/.claude/projects/<dir>/` (main session files `*.jsonl` and `<session>/subagents/*.jsonl`) with a throwaway script kept **outside the repo** (for example in `%TEMP%`), run as `python -I <script> <dir>`.
- The script prints only key paths, JSON value kinds, counts, and the values of these enum-like keys: `type`, `subtype`, `isSidechain`, `isMeta`, `origin.kind`, `turnOrigin`, `promptSource`, `permissionDecision.decision`, `permissionDecision.source`, `permissionDecision.reasonType`, `message.role`, `message.content[].type`, `message.content[].name`, `message.stop_reason`, `operation`. It never prints `text`, `thinking`, `content` strings, `input`, `toolUseResult`, `cwd`, `gitBranch`, or any other string value. For "does text start with X" questions it prints a count, not the text.
- The fixture is written by hand: real key names and enum values, invented ids (`aaaaaaaa-...` uuids, `toolu_fixture_N` tool ids), placeholder prose, and `cwd` set to the literal `__CWD__` (the test substitutes a temp git repo). Nothing is copied from a real log value.

Known from recon on 2026-11-22 (Claude Code 2.1.x, verify each and record the answer in `FORMAT.md`):
- Top-level `type` values include `user`, `assistant`, `system`, `attachment`, `queue-operation`, `file-history-snapshot`, `ai-title`, `last-prompt`, `mode`, `permission-mode`, `api-request*`, `pr-link`, `worktree-state`, `relocated`. Only `user`, `assistant`, `system` matter; everything else is ignored.
- `user`/`assistant`/`system` entries carry `uuid`, `parentUuid`, `timestamp` (RFC 3339, `Z`), `sessionId`, `isSidechain`, `cwd`, `gitBranch`.
- Human prompt: `type: user`, `origin.kind: "human"`, `turnOrigin: "human"`, `message.content` a string, `promptSource` one of `typed`, `queued`, `suggestion_accepted` (or absent). Task notification: `origin.kind: "task-notification"`, `turnOrigin: "task_notification"`. SDK turns: `turnOrigin: "sdk"`, no `origin`.
- Tool results: `type: user` with `toolUseResult` and `message.content[]` items of `type: tool_result` (`tool_use_id`, optional `is_error`); the same entry carries `permissionDecision { decision: accept|reject, source: config|user_temporary|..., reasonType }`.
- Interrupts: `type: user`, no `origin`, text content whose text begins `[Request interrupted by user` (count only; one variant appears inside a `tool_result`).
- Assistant: `message.content[]` items `text`, `thinking`, `tool_use` (`id`, `name`, `input.command` for `Bash`). One API message may be split over several entries with the same `message.id`.
- Run time: `type: system`, `subtype: turn_duration`, `durationMs`, `messageCount`. Other subtypes: `stop_hook_summary`, `away_summary`, `local_command`, `compact_boundary`, `informational`.
- Subagent files live in `<session>/subagents/agent-*.jsonl` with `isSidechain: true` on every entry.

Questions `FORMAT.md` must answer (one line each, with counts):
1. Every `permissionDecision.source` value seen, and which ones a human produced.
2. The exact interrupt shapes (top-level text vs inside `tool_result`), and whether either has `origin`.
3. For `turn_duration`: does `timestamp - durationMs` equal the turn's human prompt `timestamp` (within 1 s)? Is there exactly one per human prompt? Is it written for interrupted turns?
4. Are `uuid` values unique within a file? Across files (resumed sessions that copy history)?
5. Are files ever rewritten or truncated (compaction, `/clear`), or only appended? (Compare sizes and first lines over a day; if unknown, say so.)
6. Is the last line of a live file ever partial? (Read a file while a session writes; count lines without a trailing `\n`.)
7. Any `timestamp` not ending in `Z`, or without milliseconds?

- [ ] **Step 1:** Write the structure script in `%TEMP%` and run it over this repo's project dir and two others. Record answers in `FORMAT.md` (structure, counts, enum values only).
- [ ] **Step 2:** Write fixture session `...0001.jsonl`, in time order, one JSON object per line, timestamps from `2026-11-22T10:00:00.000Z`, containing at least:
  1. an `attachment` and a `queue-operation` line (ignored);
  2. human prompt P1 (`origin.kind: human`, prose of about 200 chars that uses a few words from the fixture repo's `README.md`, see Task 11);
  3. assistant `text` (about 600 chars) and assistant `tool_use` `Bash` with `input.command: "cargo test -p demo"`;
  4. `user` tool result for it with `is_error: true` and `permissionDecision { decision: "accept", source: "user_temporary" }`;
  5. assistant `tool_use` `Bash` `"git commit -m wip"` and its tool result with `permissionDecision { source: "config", reasonType: "mode" }`;
  6. `system` `turn_duration` closing P1;
  7. a task-notification `user` entry (`origin.kind: task-notification`) followed by an assistant `text` and its own `turn_duration`;
  8. human prompt P2, then an interrupt entry (`[Request interrupted by user]`), then `turn_duration`;
  9. a line `{"type":"user","message":` (malformed JSON) and a line with an invalid UTF-8 byte (write it with a byte escape from a script, `\xff`), both followed by `\n`;
  10. human prompt P3 with no `turn_duration` after it (an open turn), as the last complete line;
  11. a final partial line (no trailing `\n`) holding the first half of an assistant entry.
- [ ] **Step 3:** Write session `...0002.jsonl`: one human prompt with timestamps interleaved with session 1's P1 turn, an assistant text, a `turn_duration`. Write the subagent file: `isSidechain: true` entries shaped like a human prompt, a Bash `cargo test` tool use, and a tool result with `source: "user_temporary"` (all must be ignored).
- [ ] **Step 4:** Check the fixture has no real data: `git diff --cached` shows only invented ids and placeholder prose; `grep -ri "oliver\|C:/Users\|hc_bounty" crates/tally-daemon/tests/fixtures` finds nothing.
- [ ] **Step 5:** Commit: `test(daemon): Claude Code log structure notes and scrubbed fixture`.

If an answer in `FORMAT.md` contradicts a decision in "Decisions I made", stop and ask the maintainer before Task 7; update this plan's Task 7, 10 or 11 rule, not the code alone.

---

### Task 7: Line classifier

**Files:**
- Create: `crates/tally-daemon/src/claude_code/mod.rs` (`pub mod line;` for now), `crates/tally-daemon/src/claude_code/line.rs`
- Modify: `crates/tally-daemon/src/lib.rs` (`pub mod claude_code;`), `crates/tally-daemon/Cargo.toml` (`tally-scoring = { path = "../tally-scoring", features = ["text"] }`)

**Interfaces:**
- Produces (`line.rs`). Text fields live only in memory and are dropped by the caller after use:

```rust
pub struct Meta { pub uuid: String, pub session: String, pub ts: i64, pub cwd: Option<String> }
pub enum Entry {
    HumanPrompt { meta: Meta, text: String },
    Interrupt { meta: Meta },
    ToolDecision { meta: Meta, decision: tally_scoring::Decision, n: usize }, // n = index within the entry
    ToolResult { meta: Meta, tool_use_id: String, is_error: bool, text: String },
    AssistantText { meta: Meta, text: String, n: usize },
    ToolUse { meta: Meta, id: String, name: String, command: Option<String>, file_path: Option<String> },
    TurnEnd { meta: Meta },
}
#[derive(Debug, PartialEq)] pub enum LineError { Utf8, Json, Shape }
pub fn classify(line: &[u8]) -> Result<Vec<Entry>, LineError>;
pub fn parse_rfc3339_ms(s: &str) -> Option<i64>;
```

Rules (from `FORMAT.md`; correct them there first if the spike disagreed):
- Bytes not UTF-8: `Utf8`. Not a JSON object: `Json`. A `user`/`assistant`/`system` entry missing `uuid`, `sessionId` or a parseable `timestamp`: `Shape`.
- Any other `type`, and **every entry with `isSidechain: true`**: `Ok(vec![])`.
- `user` with `origin.kind == "human"` and string (or text-block) content: `HumanPrompt`. Other `origin.kind` values (task notifications) and `turnOrigin: "sdk"`: nothing.
- `user`, no `toolUseResult`, first text starting `[Request interrupted by user`: `Interrupt`.
- `user` with `tool_result` items: one `ToolResult` per item (`text` = concatenated string content); plus `ToolDecision` when `permissionDecision.source != "config"` (`accept` to `Approve`, `reject` to `Deny`; unknown decision: nothing).
- `assistant`: one `AssistantText` per `text` item; one `ToolUse` per `tool_use` item (`command` = `input.command` when it is a string; `file_path` = `input.file_path` for `Edit`, `Write`, `MultiEdit`, and `input.file_path` or `input.notebook_path` for `NotebookEdit`; confirm the key in the spike). `thinking` items are ignored.
- `system` with `subtype: "turn_duration"`: `TurnEnd`.
- `parse_rfc3339_ms`: `YYYY-MM-DDTHH:MM:SS[.fraction]Z` only (fraction truncated to ms), days-from-civil arithmetic, no dependency; anything else `None`.

- [ ] **Step 1: Write the failing tests** in `line.rs` (inline JSON strings built in the test; no fixture file needed):

```rust
#[test] fn timestamps() {
    assert_eq!(parse_rfc3339_ms("1970-01-01T00:00:00Z"), Some(0));
    assert_eq!(parse_rfc3339_ms("2026-11-22T12:34:56.789Z"), Some(1_795_350_896_789));
    assert_eq!(parse_rfc3339_ms("2026-11-22T12:34:56.7891Z"), Some(1_795_350_896_789));
    assert_eq!(parse_rfc3339_ms("2026-11-22T12:34:56+01:00"), None);
    assert_eq!(parse_rfc3339_ms("garbage"), None);
}
#[test] fn bad_bytes_and_bad_json_are_errors_not_panics() {
    assert_eq!(classify(b"{\"type\":\"user\"\xff}").err(), Some(LineError::Utf8));
    assert_eq!(classify(b"{\"type\":\"user\",\"message\":").err(), Some(LineError::Json));
    assert_eq!(classify(b"[1,2]").err(), Some(LineError::Json));
    assert_eq!(classify(br#"{"type":"user","uuid":"u"}"#).err(), Some(LineError::Shape));
}
#[test] fn human_prompt_vs_task_notification_vs_sidechain() { /* human -> [HumanPrompt]; task-notification -> []; same human line with isSidechain:true -> [] */ }
#[test] fn interrupt_is_recognised() { /* "[Request interrupted by user]" text, no origin -> [Interrupt] */ }
#[test] fn only_user_permission_decisions_count() {
    /* tool_result + permissionDecision{accept,user_temporary} -> [ToolResult, ToolDecision Approve];
       {reject,user_temporary} -> Deny; {accept,config} -> [ToolResult] only */
}
#[test] fn assistant_items_and_turn_end() {
    /* two text items + Bash tool_use + thinking -> [AssistantText n=0, AssistantText n=1, ToolUse{command:Some}];
       system turn_duration -> [TurnEnd]; system stop_hook_summary -> []; attachment -> [] */
}
#[test] fn huge_line_is_fine() {
    let text = "x".repeat(5_000_000);
    let line = format!(r#"{{"type":"user","uuid":"u","sessionId":"s","timestamp":"2026-11-22T00:00:00Z","origin":{{"kind":"human"}},"message":{{"role":"user","content":"{text}"}}}}"#);
    assert!(matches!(&classify(line.as_bytes()).unwrap()[..], [Entry::HumanPrompt { text, .. }] if text.len() == 5_000_000));
}
```

Write the bodies of the commented tests with the same style as `huge_line_is_fine`, one minimal JSON line each, asserting with `matches!`.

- [ ] **Step 2:** Run `cargo test -p tally-daemon claude_code::line`. Expected: FAIL.
- [ ] **Step 3:** Implement with `serde_json::Value` (the format is not ours and changes; typed structs would turn every new field into a decision). Run `std::str::from_utf8` before parsing.
- [ ] **Step 4:** Run `cargo test -p tally-daemon`. Expected: PASS.
- [ ] **Step 5:** Commit: `feat(daemon): classify Claude Code log lines`.

---

### Task 8: Complete-line tailer

**Files:**
- Create: `crates/tally-daemon/src/claude_code/tail.rs` (`pub mod tail;` in `mod.rs`)

**Interfaces:**
- Produces:

```rust
pub struct Chunk { pub lines: Vec<Vec<u8>>, pub start: u64, pub end: u64, pub rotated: bool }
/// Reads whole lines from `offset`. A last line without `\n` is not returned and `end` stays at its start.
/// If the file is shorter than `offset` it was truncated or replaced: reading restarts at 0 and `rotated` is true.
/// Stops after the first complete line that takes the total past `budget` bytes, so one poll's memory is
/// bounded by `budget` plus one line. A trailing `\r` is stripped. Empty lines are skipped.
pub fn read_lines(path: &std::path::Path, offset: u64, budget: usize) -> std::io::Result<Chunk>;
```

Use `BufReader::read_until(b'\n', ..)` after `seek(offset)`; a line without `\n` at EOF means "being written".

- [ ] **Step 1: Write the failing tests** in `tail.rs` (tempfile):

```rust
#[test] fn partial_last_line_waits_for_its_newline() {
    let d = tempfile::tempdir().unwrap(); let p = d.path().join("s.jsonl");
    std::fs::write(&p, b"one\ntw").unwrap();
    let c = read_lines(&p, 0, 1 << 20).unwrap();
    assert_eq!((c.lines, c.end), (vec![b"one".to_vec()], 4));
    std::fs::OpenOptions::new().append(true).open(&p).unwrap().write_all(b"o\r\nthree\n").unwrap();
    let c = read_lines(&p, 4, 1 << 20).unwrap();
    assert_eq!((c.lines, c.end), (vec![b"two".to_vec(), b"three".to_vec()], 15));
    assert_eq!(read_lines(&p, 15, 1 << 20).unwrap().lines.len(), 0);
}
#[test] fn budget_bounds_a_poll_but_never_splits_a_line() {
    let d = tempfile::tempdir().unwrap(); let p = d.path().join("s.jsonl");
    let big = "x".repeat(3_000_000);
    std::fs::write(&p, format!("{big}\na\nb\n")).unwrap();
    let c = read_lines(&p, 0, 1024).unwrap();
    assert_eq!((c.lines.len(), c.lines[0].len()), (1, 3_000_000));
    let c = read_lines(&p, c.end, 1024).unwrap();
    assert_eq!(c.lines, vec![b"a".to_vec(), b"b".to_vec()]);
}
#[test] fn a_shorter_file_is_reread_from_the_start() {
    let d = tempfile::tempdir().unwrap(); let p = d.path().join("s.jsonl");
    std::fs::write(&p, b"new\n").unwrap();
    let c = read_lines(&p, 500, 1 << 20).unwrap();
    assert!(c.rotated);
    assert_eq!((c.start, c.lines.len()), (0, 1));
}
#[test] fn non_utf8_bytes_pass_through_unchanged() {
    let d = tempfile::tempdir().unwrap(); let p = d.path().join("s.jsonl");
    std::fs::write(&p, b"\xff\xfe\nok\n").unwrap();
    assert_eq!(read_lines(&p, 0, 1 << 20).unwrap().lines.len(), 2);   // the classifier rejects it, not the tailer
}
```

- [ ] **Step 2:** Run `cargo test -p tally-daemon claude_code::tail`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test -p tally-daemon`. Expected: PASS.
- [ ] **Step 5:** Commit: `feat(daemon): complete-line log tailer`.

---

### Task 9: Prompt features: project vocabulary and context tail

**Files:**
- Create: `crates/tally-daemon/src/claude_code/features.rs` (`pub mod features;` in `mod.rs`)
- Modify: `crates/tally-scoring/src/text.rs` (add `pub fn len(&self) -> usize` and `pub fn is_full(&self) -> bool { self.len() >= VOCAB_MAX }` to `Vocab`, with one test; no ruleset change)

**Interfaces:**
- Produces:

```rust
/// Tokens of git-tracked text files under `root` (`git -C <root> ls-files -z`), most recently modified first,
/// until `Vocab::is_full()`. Skips files over 1 MiB, files containing a NUL byte in their first 8 KiB, and
/// non-UTF-8 files. `git` missing or failing gives an empty vocabulary and `Err` (the caller warns once).
pub fn project_vocab(root: &std::path::Path) -> std::io::Result<tally_scoring::text::Vocab>;
/// Last `CONTEXT_TAIL_BYTES` of conversation text (prompts and assistant text), char-boundary safe.
#[derive(Default)] pub struct ContextTail(String);
impl ContextTail { pub fn push(&mut self, text: &str); pub fn as_str(&self) -> &str; }
/// (chars, relevance_pm, novelty_bytes). Vocabulary = `project` plus `last_output` plus `last_error` (spec §5).
pub fn prompt_numbers(text: &str, project: &Vocab, last_output: &str, last_error: &str, tail: &ContextTail) -> (u32, u16, u32);
```

- `ContextTail::push` appends `"\n"` plus the text, then drops whole chars from the front until at most `CONTEXT_TAIL_BYTES` remain. It must not copy a huge input in full first: push only the last `CONTEXT_TAIL_BYTES` of `text` (on a char boundary).
- `prompt_numbers`: `chars` = `text.chars().count()` saturated to `u32::MAX`; clone `project`, `extend` with `last_output` and `last_error`, then `tally_scoring::text::prompt_features(text, &vocab, tail.as_str())`.
- Text arguments are borrowed and never stored by this module except inside `ContextTail` (memory only).

- [ ] **Step 1: Write the failing tests** in `features.rs`:

```rust
#[test] fn context_tail_keeps_the_last_8_kib_on_char_boundaries() {
    let mut t = ContextTail::default();
    t.push(&"é".repeat(10_000));                  // 20_000 bytes
    t.push("end");
    assert!(t.as_str().len() <= CONTEXT_TAIL_BYTES && t.as_str().ends_with("end"));
}
#[test] fn last_error_words_make_a_prompt_relevant() {
    let empty = Vocab::default(); let tail = ContextTail::default();
    let p = "why does borrow_checker reject moved_value here";
    let (_, without, _) = prompt_numbers(p, &empty, "", "", &tail);
    let (chars, with, nov) = prompt_numbers(p, &empty, "", "error: borrow_checker moved_value", &tail);
    assert_eq!(chars, p.chars().count() as u32);
    assert!(with > without && nov >= 24, "{with} {without} {nov}");
}
#[test] fn vocab_comes_from_tracked_files_only() {
    // tempdir; `git init`; write tracked.md "zebra_word" and untracked.md "yak_word"; `git add tracked.md`
    // let v = project_vocab(dir).unwrap();
    // relevance_pm("zebra_word", &v) == 1000 and relevance_pm("yak_word", &v) == 0
    // skip (return early with eprintln) if `git --version` fails
}
#[test] fn a_huge_prompt_is_fine() {
    let p = "word ".repeat(1_000_000);
    let (chars, _, _) = prompt_numbers(&p, &Vocab::default(), "", "", &ContextTail::default());
    assert_eq!(chars, 5_000_000);
}
```

- [ ] **Step 2:** Run `cargo test -p tally-daemon claude_code::features` and `cargo test -p tally-scoring --features text`. Expected: FAIL.
- [ ] **Step 3:** Implement. Sort `ls-files` paths by `metadata().modified()` descending (files that vanished are skipped).
- [ ] **Step 4:** Run both test commands. Expected: PASS.
- [ ] **Step 5:** Commit: `feat(daemon): project vocabulary and context tail for prompt features`.

---

### Task 10: Session state machine: events, spans, agent test runs and commits

**Files:**
- Create: `crates/tally-daemon/src/claude_code/session.rs` (`pub mod session;` in `mod.rs`)

**Interfaces:**
- Consumes: `line::{Entry, Meta}`, `features::{ContextTail, prompt_numbers}`, `tally_scoring::{Event, Kind, Actor}`.
- Produces:

```rust
pub const OPEN_SPAN_MAX_MS: i64 = 6 * 3_600_000;
/// Per-file state. Text fields are memory only; `open_turn` is persisted by the caller as "<uuid> <ts>".
#[derive(Default)]
pub struct Session {
    pub open_turn: Option<(String, i64)>,          // (prompt uuid, prompt ts)
    pub last_seen_ts: i64,
    pending: BTreeMap<String, Pending>,            // Bash tool_use id -> (meta, cmd_hash, Test|Commit)
    tail: ContextTail, last_output: String, last_error: String,
    /// (tool_use ts, file_path) of agent Edit/Write/MultiEdit/NotebookEdit calls since the last drain. Memory only.
    /// Plan 08 drains it into `Source::take_agent_writes` for its heartbeat agent-write filter (README ruling 7);
    /// until then `ClaudeCode::poll` clears it after each poll.
    /// Shape is fixed here: `(ts, raw path)`, and so is `Entry::ToolUse.file_path`. Plan 08 Task 19 turns these into hashed
    /// `take_agent_writes()` output (`(heartbeat::file_hash(project_id, rel), ts)`, via `hash_writes` called from
    /// `ClaudeCode::poll`). Plan 08 leaves this test unchanged and adds `hash_writes` tests. Until then `poll` just clears them.
    pub agent_writes: Vec<(i64, String)>,
}
pub struct Ctx<'a> { pub project: Option<String>, pub vocab: &'a Vocab }
impl Session {
    pub fn feed(&mut self, e: Entry, ctx: &Ctx, out: &mut Vec<Event>);
    /// Closes a turn open longer than OPEN_SPAN_MAX_MS at `last_seen_ts`.
    pub fn expire(&mut self, now_ms: i64, session: &str, project: Option<String>, out: &mut Vec<Event>);
    /// Highest watermark this session allows: min(open turn ts, oldest pending tool_use ts) - 1, or i64::MAX.
    pub fn limit(&self) -> i64;
}
pub fn bash_kind(command: &str) -> Option<BashKind>;   // BashKind::{Test, Commit}
pub fn cmd_hash(command: &str) -> String;              // lowercase hex SHA-256
```

`feed` rules (ids are `claude:<session>:<uuid>:<kind>[:<n>]`, `source: "claude-code"`, `session: Some(meta.session)`, `project: ctx.project`):
- `HumanPrompt`: `prompt_numbers(text, ctx.vocab, last_output, last_error, tail)` gives a `prompt` event at `meta.ts`; then `tail.push(text)`; if no turn is open, `open_turn = (uuid, ts)` (a prompt queued during a running turn joins that turn).
- `Interrupt`: `interrupt` event.
- `ToolDecision`: `tool_decision` event, id suffix `:tool_decision:<n>`.
- `AssistantText`: `agent_output { chars }` (suffix `:agent_output:<n>`); `last_output = text`; `tail.push(text)`.
- `ToolUse` with `name == "Bash"` and `bash_kind(command) = Some(k)`: store `pending[id] = (meta, cmd_hash(command), k)`. The command string is dropped here.
- `ToolResult` whose `tool_use_id` is pending: remove it; `Test` gives `test_run { cmd_hash, exit_code: is_error as i32, initiator: Agent }` at the tool_use `ts` (id from the tool_use entry: `claude:<s>:<uuid>:test_run`); `Commit` with `!is_error` gives `commit { sha: "claude:<tool_use id>", by: Agent, lines_changed: 0 }` at the tool_use `ts`. Any `ToolResult` with `is_error` sets `last_error = text`.
- `TurnEnd`: if a turn is open, `agent_busy { end_ts: meta.ts }` at the open turn's ts, id `claude:<s>:<open uuid>:agent_busy`; clear it. No open turn (task-notification turns): nothing.
- `ToolUse` with `name` in `Edit`, `Write`, `MultiEdit`, `NotebookEdit` and `Some(file_path)`: push `(meta.ts, file_path)` to `agent_writes`; no event.
- Every entry updates `last_seen_ts = max(last_seen_ts, meta.ts)`.
- `bash_kind`: trim, then `Commit` if, after splitting on `&&`, `||`, `;`, `|` and newlines, any segment is a git commit. Per segment: strip leading `(` and quote characters, `VAR=val` env-prefix tokens and a `bash -c`/`sh -c` prefix (then strip quotes again); split on whitespace; require `tokens[0] == "git"` and the first later token that is not an option (`-…`) and not the argument of `-C`/`-c` to be `commit` (so `git -C dir commit -m x`, `GIT_AUTHOR_DATE=x git commit`, `bash -c "git commit -m x"`, `(git commit -m x)` and `git add . && git commit -m x` are commits; Claude chains these often, and a missed marker would let plan 04 record the real commit as a USER commit). A quoted `echo "git commit"` is not split-safe and may count as a commit: a false positive only makes a commit agent-owned, the safe direction. Otherwise `Test` if it starts with one of `cargo test`, `cargo nextest`, `npm test`, `npm run test`, `pnpm test`, `yarn test`, `bun test`, `pytest`, `python -m pytest`, `go test`, `dotnet test`, `make test`; else `None`. (Chained commands such as `cd x && cargo test` are not matched; noted under "Not in this plan".)

- [ ] **Step 1: Write the failing tests** in `session.rs`, feeding `Entry` values built by a helper `meta(uuid, ts)` (session `"s"`):

```rust
#[test] fn a_turn_holds_the_limit_below_its_start_until_it_closes() {
    let (mut s, v) = (Session::default(), Vocab::default()); let mut out = vec![];
    let ctx = Ctx { project: Some("p".into()), vocab: &v };
    s.feed(Entry::HumanPrompt { meta: meta("u1", 1_000), text: "fix the parser".into() }, &ctx, &mut out);
    assert_eq!(s.limit(), 999);
    s.feed(Entry::AssistantText { meta: meta("u2", 5_000), text: "done".into(), n: 0 }, &ctx, &mut out);
    assert_eq!(s.limit(), 999);
    s.feed(Entry::TurnEnd { meta: meta("u3", 9_000) }, &ctx, &mut out);
    assert_eq!(s.limit(), i64::MAX);
    let busy = out.iter().find(|e| matches!(e.kind, Kind::AgentBusy { .. })).unwrap();
    assert_eq!((busy.id.as_str(), busy.ts, &busy.kind), ("claude:s:u1:agent_busy", 1_000, &Kind::AgentBusy { end_ts: 9_000 }));
}
#[test] fn turn_end_without_a_human_turn_emits_nothing() { /* TurnEnd alone -> out is empty */ }
#[test] fn a_stale_open_turn_expires_at_its_last_entry() {
    /* prompt at 0, AssistantText at 60_000, expire(now = 60_000 + OPEN_SPAN_MAX_MS + 1)
       -> AgentBusy { end_ts: 60_000 } at ts 0, limit() == i64::MAX; expire before that time -> nothing */
}
#[test] fn agent_test_run_and_commit_come_from_bash_results() {
    /* ToolUse Bash "cargo test -p x" at 2_000 id "t1" -> limit() == 1_999 while pending;
       ToolResult t1 is_error:true -> TestRun { cmd_hash: cmd_hash("cargo test -p x"), exit_code: 1, initiator: Agent } at ts 2_000;
       ToolUse "git commit -m x" at 3_000 id "t2" -> limit() == 2_999 while pending (plan 04's git source gates on this
       watermark, README ruling 1); ToolResult ok -> Commit { by: Agent, sha: "claude:t2", lines_changed: 0 } at ts 3_000;
       bash_kind("git add . && git commit -m x") == Some(Commit); bash_kind("echo \"git commit\"") == None (its segment starts
       with echo); Some(Commit) also for "git -C dir commit -m x", "GIT_AUTHOR_DATE=x git commit",
       "bash -c \"git commit -m x\"" and "(git commit -m x)"; and a quoted false positive, if any, is acceptable (agent-owned is the safe direction);
       ToolUse "ls" -> nothing pending; no event contains the command text (serde_json::to_string(&out) has no "cargo") */
}
#[test] fn agent_file_edits_are_recorded_for_plan_08() {
    /* ToolUse Edit with file_path Some("src/a.rs") at 4_000 -> agent_writes == [(4_000, "src/a.rs")], no event;
       ToolUse Read with a file_path -> nothing recorded */
}
#[test] fn prompt_features_use_the_last_error() {
    /* ToolResult (not pending) is_error text "error: zebra_word failed", then HumanPrompt "why zebra_word"
       -> prompt relevance_pm > 0 with an empty project vocab */
}
#[test] fn ids_are_stable_across_two_identical_feeds() {
    /* feed the same 6 entries into two fresh Sessions -> identical Vec<Event> */
}
```

- [ ] **Step 2:** Run `cargo test -p tally-daemon claude_code::session`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test -p tally-daemon`. Expected: PASS.
- [ ] **Step 5:** Commit: `feat(daemon): Claude Code session events and agent spans`.

---

### Task 11: The Claude Code source, project mapping, fixture end-to-end

**Files:**
- Modify: `crates/tally-daemon/src/claude_code/mod.rs`, `crates/tally-daemon/src/project.rs`, `crates/tally-daemon/src/run.rs` (`sources(cfg, home, store)` adds the Claude Code source when `cfg.claude_code.enabled`)
- Create: `crates/tally-daemon/tests/claude_code.rs`

**Interfaces:**
- Produces:

```rust
// project.rs
/// Local project id of the git repository containing `dir`, without registering it. `None` outside git
/// or when `dir` does not exist.
pub fn project_for_dir(dir: &std::path::Path) -> Option<(std::path::PathBuf, String)>;  // (root, local id)
// claude_code/mod.rs
pub const WRITE_SLACK_MS: i64 = 120_000;
pub const POLL_BUDGET: usize = 4 << 20;
pub struct ClaudeCode { /* projects_dir, sessions: BTreeMap<rel path, Session>, vocab cache, project cache */ }
impl ClaudeCode { pub fn new(projects_dir: PathBuf) -> Self; }
impl Source for ClaudeCode { fn id(&self) -> &str { "claude-code" } /* poll below */ }
```

`register_git_project` is refactored to call `project_for_dir` so both use the same normalization.

`poll(now_ms, state)` (the part the signatures do not determine):
1. Files: `projects_dir/*/*.jsonl` only (one level; `<session>/subagents/` is never read). The state key suffix `<rel>` is the path relative to `projects_dir` with `/` separators.
2. Live capture: if `state.get("started")` is `None`, set every existing file's `off:<rel>` to its current length, set `started = now_ms`, and read nothing this poll. Files without `off:` later are new sessions and start at 0.
3. Warm-up (once per file per process, when `off > 0`): re-read the last 64 KiB before `off` (skip to the first `\n`), feed the entries to the file's `Session`, **discard** the events. This rebuilds the context tail and pending tool uses after a restart without storing text. Restore `open_turn` from `open:<rel>` if present.
4. `read_lines(path, off, POLL_BUDGET)`. If `rotated`: drop the file's `Session`, and from then on drop emitted events with `ts <= last_ts:<rel>` (they were already sent; re-sending one with a new id at or before `finalized_through` would make the whole batch late).
5. Each line: `classify`; an `Err` increments a per-poll skip counter (printed with `eprintln!` as `source claude-code: skipped <n> unreadable lines`, never with line content) and the line is consumed. `Ok(entries)`: for each, `Ctx { project, vocab }` where `project`/vocab come from `meta.cwd` through `project_for_dir` (cached per cwd) and `project_vocab(root)` (cached per root, rebuilt when older than 10 minutes; an `Err` is printed once per root).
6. Every session: `expire(now_ms, ..)`.
7. Write state: `off:<rel> = chunk.end`, `last_ts:<rel>` = max event ts sent, `open:<rel>` = `"<uuid> <ts>"` or `"-"` when closed.
8. Watermark: `min(now_ms - WRITE_SLACK_MS, min over sessions of limit())`. Return `Poll { events, watermark: Some(w) }`. (`WRITE_SLACK_MS` covers entries whose timestamp is earlier than the moment the line is flushed; it also covers a partial last line, whose timestamp is not yet known.)

- [ ] **Step 1: Write the failing end-to-end test** in `tests/claude_code.rs`. Setup helper: a tempdir git repo (`git init`, `README.md` with the words used by the fixture prompts, `git add`, `git -c user.name=t -c user.email=t@t commit -m init`; return early with `eprintln!` if `git` is missing), a tempdir `projects` into which the fixture tree from Task 6 is copied with every `__CWD__` replaced by the repo path (forward slashes, JSON-escaped), and `<home>/config.toml` pointing `projects_dir` at it. Run the binary as in `tests/cli.rs`.

```rust
#[test] fn fixture_sessions_end_to_end() {
    // 1. `run --once` with an EMPTY projects dir: records "started" (live capture), "added 0, ..."
    // 2. copy the fixture tree in; `run --once`: exit 0; stderr contains "skipped 2 unreadable lines"
    // 3. open <home>/tally.db with rusqlite; count events by kind (json_extract(json,'$.kind')):
    //    prompt 4 (P1, P2, P3, session 2), interrupt 1, tool_decision 1 (the user_temporary accept only),
    //    test_run 1 (exit_code 1, initiator agent), commit 1 (by agent), agent_busy 3 (P1, P2, session 2;
    //    none for the task-notification turn, none for open P3); nothing from the subagent file
    //    (no event id contains "a000000000000001" and no second test_run)
    // 4. every prompt event has project == the repo's local id (`tally project add` is NOT run: unnamed id)
    // 5. horizon (SELECT ts FROM watermarks WHERE source='claude-code') < P3's ts
    // 6. no text on disk: the db file bytes contain none of the fixture prompt sentences or "cargo test"
    // 7. `run --once` again: stdout starts "added 0,"
    // 8. complete the partial last line (append its second half + "\n"): `run --once` adds exactly 1 event
    // 9. append a turn_duration for P3: `run --once` adds the P3 agent_busy; horizon moves past P3's ts
    // 10. re-read from scratch: UPDATE source_state SET value='0' WHERE key LIKE 'off:%'; `run --once`:
    //     stdout starts "added 0," (all duplicates)
}
#[test] fn a_rotated_file_is_reread_without_duplicates() {
    // after step 2 above, rewrite session 2's file with only its first two lines (shorter than the offset):
    // `run --once` exits 0 and adds 0 events
}
```

Also a unit test in `project.rs`: `project_for_dir(sub)` of a tempdir with `.git` equals `(root, project_id(root))`, and is `None` for a nonexistent path.

- [ ] **Step 2:** Run `cargo test -p tally-daemon --test claude_code`. Expected: FAIL.
- [ ] **Step 3:** Implement `project_for_dir`, `ClaudeCode`, and the Claude Code entry in `run::sources`.
- [ ] **Step 4:** Run `cargo test -p tally-daemon` and `cargo clippy -p tally-daemon --all-targets`. Expected: PASS, no warnings. CI (`.github/workflows/daemon.yml`) runs on three OSes; check `git` is on `PATH` there (it is on GitHub runners).
- [ ] **Step 5: Manual check on the maintainer's machine** (record results in the PR description, numbers only): `tally project add C:/Users/Oliver/hc_bounty/tally --name sectersion/tally`; `tally run --interval-secs 15`; work in Claude Code for 10 minutes; Ctrl-C prints `stopped` after the current tick; `tally status` shows events and a horizon; `tally verify` prints `ok`. While `tally run` is running, `tally status` in a second terminal works (WAL).
- [ ] **Step 6:** Commit: `feat(daemon): Claude Code session-log source`.

---

## Done when

- `cargo test -p tally-daemon` and `cargo test -p tally-scoring --features text` pass on Windows, macOS and Linux in CI.
- The fixture end-to-end test shows the expected event counts, idempotent re-reads, partial-line completion, rotation without duplicates, an open span holding the horizon, and no text in the database.
- On the maintainer's machine, `tally run` captures a real Claude Code session, Ctrl-C stops it cleanly, and `tally verify` passes.

## Not in this plan

- Idle spans, the git source (real commits, user commits, user-initiated test runs), worktree/clone matching to one public name: plan 04. Until plan 04, the scorer treats missing idle data as presence (README ruling); that is why the badge waits for plan 04.
- Publishing: plan 05. Other harnesses and the heartbeat endpoint: plan 08.
- Chained shell commands (`cd x && cargo test`), test runs inside subagents, user-edited files.
- Backfilling sessions from before the daemon first ran (ruleset change; README ruling).
- Service registration and autostart: plan 12.
