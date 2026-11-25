# More Adapters, Adapter Health, and the Heartbeat Endpoint Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** (1) A shared adapter test kit and `tally doctor`, so every adapter is tested against committed fixtures from each supported log version and the user can see per harness whether logs were found, when the last event was seen, the parse error rate and the detected format version. (2) Adapters for Codex CLI, Aider, Gemini CLI, GitHub Copilot and Cursor (through `tally hook`). (3) A localhost WakaTime-compatible heartbeat endpoint whose surviving heartbeats become `user_edit` events.

**Architecture:** Each adapter is a `Source` (index contract C1) split in two layers: a pure `parse` function (log text in, `Vec<Parsed>` plus a `ParseReport` out, no I/O, no clock) and a thin poll wrapper (file discovery, byte-offset cursors in `SourceState`, watermark). Only the pure layer is golden-tested; the wrapper is shared code in `adapters/common.rs`. Sources never touch the store (README C1, ruling 5): adapter health and agent writes leave a source only through two new default `Source` methods, `take_health()` and `take_agent_writes()`, which the runner in `run.rs` persists, exactly like plan 04's `take_coverage()`. The heartbeat server is a small blocking `tiny_http` server on a thread; its heartbeats become `user_edit` events returned by `HeartbeatSource::poll` like any other source's.

**Tech Stack:** Rust stable, `tally-scoring`, existing daemon crate, plus `tiny_http = "0.12"` (heartbeat server), `base64 = "0.22"` (auth header), `serde` with derive, and plan 05's `chrono` (timestamps with offsets). No async runtime.

**Spec:** `docs/superpowers/specs/2026-11-22-daemon-roadmap-design.md` (section 5), `docs/spec/scoring.md` (§2 events, §3 hours, §4 caps), `PITCH.md` (passive capture, "Harness log formats change" known limit), `docs/superpowers/plans/README.md` (contracts C1 to C5).

## Assumes from earlier plans

If the code on `main` differs from the names below, the code wins: change the names here and note it in the task commit.

- From plan 03 (`crates/tally-daemon/src/source.rs`): `struct Poll { events: Vec<Event>, watermark: Option<i64> }`, `trait SourceState { fn get(&self, key: &str) -> Option<String>; fn set(&mut self, key: &str, value: &str); }`, `SourceError { Io, Other(String) }`, and `trait Source` with plan 04's default method `take_coverage`. After Task 1 of this plan the trait is:

```rust
pub trait Source {
    fn id(&self) -> &str;
    fn poll(&mut self, now_ms: i64, state: &mut dyn SourceState) -> Result<Poll, SourceError>;
    fn take_coverage(&mut self) -> Vec<Interval> { Vec::new() }       // plan 04
    fn take_health(&mut self) -> Option<Health> { None }              // this plan, Task 1
    fn take_agent_writes(&mut self) -> Vec<(String, i64)> { Vec::new() } // this plan, Task 1: (file_hash, ts)
    fn see_agent_writes(&mut self, _writes: &[(String, i64)]) {}       // this plan, Task 18: runner forwards the tick's agent writes
}
```

- From plan 03 (`crates/tally-daemon/src/run.rs`, README ruling 6): `run_once`, `run_loop(store, cfg, home, stop)` and `sources(cfg, home, store) -> Vec<Box<dyn Source>>`, which builds the source list from `<home>/config.toml` (C5; each source reads its own table; unknown keys are an error). Every source in this plan is registered in `sources()`, never in `main.rs`. Also the `source_state` table and WAL mode.
- From plan 03 (`crates/tally-daemon/src/claude_code/`): `features::{project_vocab(root: &Path) -> io::Result<Vocab>, ContextTail, prompt_numbers(text: &str, project: &Vocab, last_output: &str, last_error: &str, tail: &ContextTail) -> (u32 /*chars*/, u16 /*relevance_pm*/, u32 /*novelty_bytes*/)}`; `line::{classify, Entry, Meta, parse_rfc3339_ms}`; `session::Session` (the per-session state machine that turns `line::Entry::ToolUse` into events); `WRITE_SLACK_MS = 120_000` in `claude_code/mod.rs`; source id `claude-code`.
- From plan 03: stable event ids `<source>:<native-id>:<kind>` (C2) and the rule "human prompts only, never agent-originated turns".
- From plan 03: `project::project_for_dir(dir: &Path) -> Option<(PathBuf /*root*/, String /*local id*/)>`, the function the Claude adapter maps a session cwd with. Every adapter here uses it (through `ParseCtx`, Task 1). Sessions whose cwd is in no git repository emit events with `project: None`, as the Claude adapter does.
- From plan 04: `tally hook` is a clap subcommand group, `Cmd::Hook { #[command(subcommand)] cmd: HookCmd }` with `HookCmd::TestRun` (README ruling 12). This plan adds `HookCmd::Cursor { event }` and `HookCmd::Install { harness }` to that enum.
- From plan 04: the git source emits `commit` events and test-run capture emits `test_run`. Adapters here emit agent `test_run`/`commit` only when the harness log itself records the agent running a test command or committing; the git source stays the owner of user commits.
- From code on `main` (verified): `Store::{append_events, set_watermark, register_project, project_names}`, `ingest_ndjson`, `project::{normalize_path, project_id, find_git_root, register_git_project}`, `tally_scoring::{Event, Kind, Decision, Actor}` with `Kind::{Prompt{chars,relevance_pm,novelty_bytes}, Interrupt, ToolDecision{decision}, UserEdit{file_hash,lines_changed}, TestRun{cmd_hash,exit_code,initiator}, Commit{sha,by,lines_changed}, AgentBusy{end_ts}, Idle{end_ts}, AgentOutput{chars}}`. `main.rs` is a clap enum `Cmd` with `--home`/`TALLY_HOME`.

## Global Constraints

- Never invent a log format. Every adapter starts with a spike task that produces a committed, scrubbed fixture and a `NOTES.md` stating what was observed on a real machine and what was only read from vendor documentation. A parser is written only against a fixture. Where a fact could not be verified the notes say `UNVERIFIED` and the parser treats that part leniently (skip and count in `ParseReport.skipped`, never guess).
- Human prompts only. A user message the harness generated (tool result echoed as a user turn, system reminder, compaction summary, slash-command expansion, notification) is not a prompt. If the log cannot distinguish them, the adapter emits no `prompt` for that harness and the doctor says so. Under-counting is acceptable; over-counting is not.
- Only derived numbers leave the adapter (index C3). Prompt text, assistant text, file paths, commands and file contents are read into memory, reduced to `chars`, `relevance_pm`, `novelty_bytes`, `cmd_hash`, `file_hash`, and dropped. No `Debug` print or error message may contain log text: errors carry file name, line number and a category only.
- Stable ids (C2): `<source-id>:<native-id>:<kind>`; where the log has no native id, `<source-id>:<session>:<record-index>:<kind>` using position data that survives appends. A log re-read from byte 0 must yield the same ids.
- Source ids are exactly: `codex`, `aider`, `gemini-cli`, `copilot`, `cursor`, `heartbeat`. The `source` field of every event equals the source id.
- Watermark rule (C1): a poll returns watermark `w` only when no later poll can emit an event with `ts <= w`. An open `agent_busy` span (a turn with a start and no end yet) keeps `w` below that span's start. Every log-reading source follows plan 03's write-slack rule: `w = min(now_ms - WRITE_SLACK_MS, open span starts - 1)` (`common::file_watermark`, Task 1), never `now_ms - 1` or the newest event `ts`, because a harness may flush a line seconds after its timestamp and one event at or below `finalized_through` makes the runner drop the whole batch (`LateEvent`).
- Log timestamps may be local time or ISO-8601; convert to UTC ms. A record without a usable timestamp is skipped and counted, never stamped with `now`.
- Heartbeat server binds `127.0.0.1` only, rejects any `Host` header whose host is not `127.0.0.1`, `localhost` or `[::1]` with `403`, caps body size and bulk length, and stores no file path or file contents.
- Windows first; everything compiles and tests pass on Windows, macOS and Linux. Fixtures keep LF endings: `.gitattributes` gets `crates/tally-daemon/tests/fixtures/** text eol=lf`.
- Time values are `i64` ms; no floats in events.

## Review Focus

1. **Two harnesses active at once on one project.** Expected: each source has its own watermark and cursor state; ids are namespaced by source so nothing collides; the scorer already merges human actions across sources (spec §3), so two prompts seconds apart cause no double credit. Test: Task 2 `two_harnesses_one_project_do_not_double_credit` (scores two fixtures merged and asserts seconds are less than the sum of each alone), plus a poll test with two sources sharing one `Store`.
2. **A session log replayed after an app update changes the format.** Expected: the parser detects the format version per file, parses what it can, counts unknown records in `ParseReport.skipped` and unparsable ones in `ParseReport.errors`, never panics, never guesses a timestamp; the same record gets the same id before and after; an event with an already stored id is a duplicate (ignored). `tally doctor` shows the version as `unknown` and a non-zero error rate. A file whose earlier bytes changed (rewritten, not appended) is detected by a stored prefix hash and re-read from 0 (duplicates are free). Tests: Task 2 `rewritten_file_is_reread`, each adapter's `*_format_drift` test.
3. **Heartbeats arriving out of order or in bulk.** Expected: accepted in any order; a heartbeat at or below the source's own watermark (which `finalized_through` never exceeds) is answered `201` (the plugin must not retry forever) and counted in `late_dropped`, since the store would reject the whole batch with `LateEvent`; each heartbeat is stored or dropped independently; the heartbeat source watermark is `now - HEARTBEAT_SLACK_MS`, never the newest heartbeat `ts`, so a delayed older heartbeat is still accepted. Tests: Tasks 17 and 18.
4. **A heartbeat for a file outside any project (no git root, via `project::project_for_dir` as for Claude Code).** Expected: answered `201`, the path is not stored, no event is emitted (a `user_edit` with `project: None` would claim time attributed to nothing), counted in `outside_project`. Test: Task 18.
5. **Agent-caused heartbeats.** Expected: a heartbeat within `AGENT_WRITE_WINDOW_MS` after an agent write to the same file hash is discarded; a heartbeat for another file in that window survives; one for the same file after the window survives. Claude Code (the main harness) reports its Edit/Write/MultiEdit/NotebookEdit writes too (Task 19), so agent activity never moves the badge. Tests: Tasks 18 and 19.
6. **Host header and binding.** Expected: `Host: evil.example` gets `403`; the listener address is `127.0.0.1`. Test: Task 17.

## File Structure

```
crates/tally-daemon/
  Cargo.toml                          add tiny_http, base64, serde (derive)
  src/lib.rs                          pub mod adapters; pub mod doctor; pub mod hook; pub mod heartbeat;
  src/source.rs                       (plan 03) Health, default methods take_health, take_agent_writes
  src/run.rs                          (plan 03) persist take_health / take_agent_writes; register sources in sources()
  src/store.rs                        adapter_health and agent_writes tables
  src/claude_code/mod.rs              (plan 03) poll hashes Session.agent_writes, take_agent_writes (Task 19)
  src/adapters/mod.rs                 Parsed, ParseReport, Adapter trait, discovery helpers
  src/adapters/common.rs              FileCursor poll wrapper shared by file adapters, ids, time parsing, file_watermark
  src/adapters/codex.rs               Codex CLI parser + Source
  src/adapters/aider.rs               Aider parser + Source
  src/adapters/gemini.rs              Gemini CLI parser + Source
  src/adapters/copilot.rs             Copilot parser + Source
  src/adapters/cursor.rs              Cursor hook-spool parser + Source
  src/hook.rs                         `tally hook cursor <event>`: stdin JSON in, spool line out
  src/doctor.rs                       per-harness health report
  src/heartbeat/mod.rs                types, auth, parse, agent-write filter, mapping to events
  src/heartbeat/server.rs             tiny_http server, Host check, limits
  src/main.rs                         add `doctor`, `heartbeat-config`, `HookCmd::{Cursor, Install}` (CLI parsing only)
  tests/fixtures/<harness>/<version>/ scrubbed logs + expected.json + NOTES.md
  tests/golden.rs                     golden helper + per-fixture tests
  tests/heartbeat.rs                  HTTP test against the real server on an ephemeral port
.gitattributes                        fixtures keep LF
docs/adapters/<harness>.md            user-facing: where logs live; Cursor hook install
```

## Decisions I made (maintainer to confirm)

1. **Order and honesty.** Codex first (installed on this machine, so the spike can capture a real sample), then Aider, Gemini CLI, Copilot, Cursor. Cursor is the only adapter that needs the user to install something (a hook entry); Copilot's prompt/turn distinction is the least certain. Each can ship independently; the doctor reports `unsupported` rather than guessing.
2. **Adapter shape.** Pure `parse` plus shared poll wrapper (`common.rs`), so the golden kit tests every adapter the same way and a format change touches one function.
3. **Cursor via a spool.** `tally hook cursor <event>` appends one derived-numbers-only line to `<home>/spool/cursor.ndjson`; the `cursor` source reads the spool. The hook never talks to the database, so a locked DB cannot make Cursor wait, and the hook cannot leak prompt text (it reduces before writing).
4. **Agent-write rule for heartbeats.** Discard a heartbeat when an agent write event for the same `file_hash` exists in the previous `AGENT_WRITE_WINDOW_MS = 10_000` ms, or in the following `HEARTBEAT_SLACK_MS` window once the agent write is seen later (the check runs at flush time, after a hold of `HEARTBEAT_HOLD_MS = 15_000`, so a late-arriving agent log line still matches). Agent writes come from every agent source, plan 03's Claude Code source included (Task 19), through `Source::take_agent_writes()`. The runner stores them in a small `agent_writes(file_hash, ts)` table (never an event, survives restarts) and forwards each tick's writes to every source through `Source::see_agent_writes()`; `HeartbeatSource` keeps them in memory, so no source touches the store.
5. **Heartbeat to `user_edit`.** One `user_edit` per file per 2-minute bucket (heartbeats are every few seconds while typing; the spec cap for `user_edit` is 180 s so more events add nothing). `lines_changed` = `lines` delta from the previous heartbeat for that file when `lines` is present, else 1.
6. **No `tiny_http` async stack.** A blocking server on one thread; the load is a few requests a minute.

## Questions only the maintainer can answer

- Should an editor heartbeat for a file outside any git repository ever count? This plan says no (dropped, counted). Inside any git repository it maps by `project::project_for_dir`, the same rule plan 03 uses for Claude Code sessions, whether or not the project was added with `tally project add`.
- Is it acceptable that Cursor requires one-time hook installation (not zero-config)? There is no passive log for Cursor chat that this plan can verify.

---

### Task 1: Adapter types, shared poll wrapper, fixture layout

**Files:**
- Create: `crates/tally-daemon/src/adapters/mod.rs`, `crates/tally-daemon/src/adapters/common.rs`, `.gitattributes` (or append), `crates/tally-daemon/tests/fixtures/README.md` (three lines: layout, scrub rule, how to add a version)
- Modify: `crates/tally-daemon/src/lib.rs` (`pub mod adapters;`), `crates/tally-daemon/Cargo.toml` (`chrono = { version = "0.4", default-features = false, features = ["clock"] }`, the line plan 05 adds; skip if already present), `crates/tally-daemon/src/source.rs` (plan 03)

**Interfaces (`source.rs`, additions to plan 03's trait; the full trait is under "Assumes"):**

```rust
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Health {
    pub files_found: usize, pub records: usize, pub skipped: usize, pub errors: usize,
    pub format: String,                 // "unknown" when not recognized
    pub last_event_ts: Option<i64>,
    pub counters: Vec<(String, u64)>,   // source-specific, e.g. heartbeat accepted/dropped_agent
}
// in trait Source, next to plan 04's take_coverage:
/// Drains the health of the last poll. `None` = unchanged. The runner persists it (Task 3).
fn take_health(&mut self) -> Option<Health> { None }
/// Drains agent file writes seen by the last poll as (file_hash, ts). The runner persists them (Task 18).
fn take_agent_writes(&mut self) -> Vec<(String, i64)> { Vec::new() }
```

Sources that do not care ignore both. Neither is ever an event, and neither is called by a source on the store: sources never touch the store (C1, ruling 5).

**Interfaces (`adapters/mod.rs`):**

```rust
pub struct Parsed { pub events: Vec<tally_scoring::Event>, pub agent_write_hashes: Vec<(String /*file_hash*/, i64 /*ts*/)> }
#[derive(Default, Debug, PartialEq, Eq)]
pub struct ParseReport {
    pub format: String,        // detected version label, "unknown" when not recognized
    pub records: usize,        // records examined
    pub skipped: usize,        // recognized as not relevant, or unknown record types
    pub errors: usize,         // malformed: bad JSON, missing timestamp, unparsable
    pub last_ts: Option<i64>,  // newest timestamp among emitted events
}
pub trait Adapter {
    const ID: &'static str;
    /// Pure: whole text of one log (or the unread tail), no I/O, no clock.
    /// `ctx` supplies project resolution and prompt features so tests can stub them.
    fn parse(text: &str, ctx: &dyn ParseCtx) -> (Parsed, ParseReport);
}
pub trait ParseCtx {
    /// Production (`RealCtx`): `project::project_for_dir(Path::new(cwd))`, cached per cwd.
    fn project_for_dir(&self, cwd: &str) -> Option<(std::path::PathBuf, String)>;
    /// Production: `features::prompt_numbers(text, &vocab, last_output, last_error, tail)` with
    /// `vocab = features::project_vocab(root)` (cached per root, as plan 03 does; empty `Vocab` when `root` is None).
    /// `last_output` = the preceding assistant text, `last_error` = the preceding failed tool result text (spec §5).
    /// Returns (chars, relevance_pm, novelty_bytes).
    fn prompt_features(&self, root: Option<&std::path::Path>, text: &str, last_output: &str, last_error: &str, tail: &ContextTail) -> (u32, u16, u32);
}
```

Each parser keeps a `features::ContextTail` per session (push prompt and assistant text, as plan 03's `Session` does) plus `last_output`/`last_error`, all in memory only.

`common.rs`: `pub struct FileCursor` stored in `SourceState` under key `cursor:<normalized path>` as `"<offset>:<prefix_hash16>"` where `prefix_hash16` is the first 16 hex chars of SHA-256 over the first `min(offset, 4096)` bytes. Functions: `read_new(path, state) -> io::Result<Option<String>>` returns text from the stored offset up to the last complete line (`\n`), or the whole file when the prefix hash no longer matches (rewritten) or the file is shorter than the offset (truncated); updates nothing until `commit_cursor(state, path, new_offset, prefix)` is called after a successful append. `pub fn stable_id(source: &str, native: &str, kind: &str) -> String` returning `format!("{source}:{native}:{kind}")`. `pub fn iso_to_ms(s: &str) -> Option<i64>`: `claude_code::line::parse_rfc3339_ms(s)` for `Z` timestamps, else `chrono::DateTime::parse_from_rfc3339(s).ok().map(|d| d.timestamp_millis())` (no new date crate). `pub fn file_watermark(now_ms: i64, open_starts: impl IntoIterator<Item = i64>) -> i64` = `min(now_ms - claude_code::WRITE_SLACK_MS, min(open_starts) - 1)`: the only watermark every file adapter returns (Global Constraints).

Each adapter source keeps `health: Option<Health>` and `agent_writes: Vec<(String, i64)>` fields, fills them during `poll` from the `ParseReport` and `Parsed.agent_write_hashes`, and implements `take_health`/`take_agent_writes` with `std::mem::take`.

- [ ] **Step 1: Write the failing tests** in `common.rs`: `read_new_returns_only_complete_lines` (file `a\nb\npart`, first read returns `a\nb\n`, cursor at 4; after appending `ial\n` the next read returns `partial\n`), `rewritten_file_is_reread` (change a byte inside the first 4096 bytes and keep length: read returns the whole file), `truncated_file_is_reread`, `iso_to_ms_handles_offset_and_z` (`2026-11-23T10:00:00Z` and `2026-11-23T12:00:00+02:00` give the same ms), `file_watermark_keeps_write_slack` (`file_watermark(1_000_000, [])` is `880_000`; with an open start at `500_000` it is `499_999`), `stable_id_format`. In `source.rs`: `default_take_methods_are_empty` (a plan 03 test `Fake` source returns `None` and an empty vec).
- [ ] **Step 2:** `cargo test -p tally-daemon common`. Expected: FAIL.
- [ ] **Step 3:** Implement. A partial last line is never returned (the harness may be mid-write).
- [ ] **Step 4:** Run again. Expected: PASS, no warnings.
- [ ] **Step 5:** Commit `feat(daemon): adapter types and shared file cursor`.

---

### Task 2: Golden-test helper and format-drift kit

**Files:**
- Create: `crates/tally-daemon/tests/golden.rs`, `crates/tally-daemon/tests/fixtures/_selftest/v1/{log.txt,expected.json}`
- Modify: `crates/tally-daemon/src/lib.rs` (add `pub mod test_kit;` behind `#[cfg(any(test, feature = "test-kit"))]`; add feature `test-kit = []`; integration tests enable it with `required-features` or `cargo test --features test-kit`; CI command in Task 20 does this)
- Create: `crates/tally-daemon/src/test_kit.rs`

**Interfaces (`test_kit.rs`):**

```rust
pub struct StubCtx;                                  // project_for_dir: Some(("C:/tallyproj", "proj")) for any cwd containing "tallyproj", else None; prompt_features: (chars, 500, chars) deterministic
pub fn normalize(events: &[Event]) -> serde_json::Value;   // ids replaced by "e1","e2",... in (ts, id) order; ts replaced by ms since the first event ("t": 0, 1500, ...); all other fields kept
pub fn golden<A: Adapter>(fixture_dir: &str);        // reads <dir>/*.{jsonl,json,md,ndjson,txt} as `log` (first file), runs A::parse, compares normalize(events) to <dir>/expected.json
pub fn assert_no_text_leak(events: &[Event], forbidden: &[&str]);  // serializes events and asserts none of the forbidden substrings (prompt words, a path) appear
```

Regenerate expected output with `TALLY_BLESS=1 cargo test ...` (writes `expected.json`; reviewer reads the diff). `golden` also asserts `report.errors == 0` unless the dir contains `allow_errors` (a file with the allowed count).

- [ ] **Step 1: Write the failing tests** in `golden.rs` using a tiny self-test adapter defined in the test file (`struct Mini; impl Adapter for Mini` parsing lines `ts,kind` into events): `normalize_replaces_ids_and_rebases_time`, `golden_passes_on_matching_expected`, `bless_writes_expected` (temp dir copy), `no_text_leak_catches_a_substring`, `two_harnesses_one_project_do_not_double_credit` (two `Mini`-style event lists for the same project, prompts 5 s apart, scored with `tally_scoring::score`; assert `merged_verified_s < alone_a + alone_b`).
- [ ] **Step 2:** Run `cargo test -p tally-daemon --features test-kit --test golden`. Expected: FAIL.
- [ ] **Step 3:** Implement `test_kit.rs`.
- [ ] **Step 4:** Run again. Expected: PASS.
- [ ] **Step 5:** Commit `test(daemon): golden-test kit for adapters`.

---

### Task 3: `tally doctor` (harness health check)

**Files:**
- Create: `crates/tally-daemon/src/doctor.rs`
- Modify: `crates/tally-daemon/src/lib.rs`, `crates/tally-daemon/src/main.rs` (`Cmd::Doctor { #[arg(long)] json: bool }`), `crates/tally-daemon/src/store.rs` (table `adapter_health(source TEXT PRIMARY KEY, found_files INTEGER, records INTEGER, skipped INTEGER, errors INTEGER, format TEXT, last_event_ts INTEGER, counters TEXT /*JSON*/, checked_ts INTEGER)` with `put_health(source: &str, h: &Health, checked_ts: i64)`/`get_health() -> Vec<(String, Health, i64)>`; only the runner calls `put_health`), `crates/tally-daemon/src/run.rs` (plan 03: after every `poll`, Ok or Err, `if let Some(h) = source.take_health() { store.put_health(id, &h, now_ms) }`; a `put_health` error is pushed to `TickReport.errors` and changes nothing else)

**Interfaces:**

```rust
pub struct HarnessHealth { pub source: String, pub log_dir: Option<String> /*directory only, never a file with a session name*/, pub files_found: usize, pub last_event_ts: Option<i64>, pub records: usize, pub errors: usize, pub skipped: usize, pub format: String, pub status: Status }
pub enum Status { Ok, NoLogs, Degraded /*error rate >= 5%*/, FormatUnknown, Unsupported /*see honesty notes*/, NotInstalled }
pub fn error_rate_pm(h: &HarnessHealth) -> u32;   // 1000 * errors / max(records, 1)
pub fn report(store: &Store) -> Result<Vec<HarnessHealth>, StoreError>;
```

Each adapter's `poll` accumulates a `ParseReport` into its `Health` (rolling totals kept in `source_state` keys `health:records`, `health:errors`, `health:skipped`) and hands it out through `take_health()`; the runner persists it with `put_health`. Adapters never call the store. `doctor` reads the table and the config; it never parses logs itself. Exit code 0 always (informational); `--json` prints the structs for the future dashboard.

- [ ] **Step 1: Write the failing tests** in `doctor.rs`: `error_rate_is_per_mille`, `status_degraded_at_five_percent` (49/1000 is Ok, 50/1000 is Degraded), `no_files_means_no_logs`, `unknown_format_beats_ok` (format `"unknown"` gives `FormatUnknown` even at 0 errors), `report_lists_every_configured_source_even_without_a_row` (a configured source with no row prints `NoLogs`). In `run.rs`: `runner_persists_take_health` (a `Fake` source whose `take_health` returns `Some(Health { records: 3, .. })`: after `run_once`, `get_health()` has that row; a second tick where it returns `None` leaves the row unchanged).
- [ ] **Step 2:** `cargo test -p tally-daemon doctor`. Expected: FAIL.
- [ ] **Step 3:** Implement; add the `Doctor` command printing one line per harness: `codex  ok  files=3  last=2026-11-23T10:00:00Z  format=rollout-v1  errors=0/412`.
- [ ] **Step 4:** Run tests. Expected: PASS. Also add `tests/cli.rs::doctor_prints_a_line_per_configured_source` driving the binary with a temp home.
- [ ] **Step 5:** Commit `feat(daemon): tally doctor`.

**Note.** `last_event_ts` is the newest emitted event `ts`; "last seen" for a harness whose logs exist but whose newest record is old is shown as an age so the user sees a stale harness (for example after an update moved the log directory).

---

### Task 4: Codex CLI spike (fixture and notes)

**Files:**
- Create: `crates/tally-daemon/tests/fixtures/codex/<version>/{rollout.jsonl,NOTES.md}`, `docs/adapters/codex.md`

State on the plan author's machine (2026-11-23): `codex-cli 0.142.4` is installed (npm global) but `~/.codex` contains only `tmp/`; no session has ever been run, so there is no real sample yet. The spike therefore starts by creating one.

- [ ] **Step 1:** Run a short real session in a scratch git repo: `codex` (interactive), send two prompts, approve one command and deny one, interrupt once (Esc), exit. Also run `codex exec "say hi"` once (non-interactive; its log may differ).
- [ ] **Step 2:** Find the logs: `Get-ChildItem -Recurse $HOME\.codex | Sort LastWriteTime -Desc | Select -First 10`. Expected from memory (UNVERIFIED, confirm or correct): `~/.codex/sessions/YYYY/MM/DD/rollout-<timestamp>-<uuid>.jsonl`, one JSON object per line with top-level `timestamp`, `type` (for example `session_meta`, `response_item`, `event_msg`, `turn_context`) and `payload`. `CODEX_HOME` overrides `~/.codex`. Record in `NOTES.md`: exact path on Windows, and from vendor docs (https://github.com/openai/codex, the `docs/` and the `codex-rs` rollout recorder source) the macOS and Linux paths (same `~/.codex` unless `CODEX_HOME`), the record types seen, which record carries a human prompt versus injected context (for example environment context or AGENTS.md text delivered as a user-role message), how an approval or denial of a command appears, how an interrupt (turn aborted) appears, how a turn start and end appear, and the format version (`session_meta` payload version or CLI version). Mark each item `OBSERVED` or `UNVERIFIED`.
- [ ] **Step 3:** Scrub into a fixture: keep structure and timestamps relative; replace every prompt and assistant text with `"x"` repeated to the same length (so `chars` stay meaningful), every path with `C:/tallyproj/f1.rs` style, every command with `cmd1` (keep exit codes), uuids with fixed fakes. Run `rg -i "<your name>|<home dir>|api[_-]?key|token" fixture` and expect no hits. Commit the fixture as `rollout.jsonl` under a directory named by the detected version.
- [ ] **Step 4:** Make a second fixture, `drift/rollout.jsonl`, by hand: the same log with one record type renamed, one line truncated mid-JSON, and one line missing its timestamp. It is the drift test input (documented as synthetic in `NOTES.md`).
- [ ] **Step 5:** Commit `test(daemon): codex fixtures and format notes`. If Steps 1 or 2 cannot be done (Codex cannot run, no login), stop this task and the next two, commit only `NOTES.md` with what is verifiable from documentation marked `UNVERIFIED`, and report it; do not write the parser.

---

### Task 5: Codex parser

**Files:**
- Create: `crates/tally-daemon/src/adapters/codex.rs`
- Modify: `crates/tally-daemon/src/adapters/mod.rs`

**Interfaces:** `pub struct Codex; impl Adapter for Codex { const ID: &str = "codex"; fn parse(..) }`. Internal: one `serde_json::Value` per line (never deserialize into a strict struct: unknown fields must not fail). Session id and cwd come from the session header record; project root and id from `ctx.project_for_dir(cwd)`.

**Mapping rules** (fill the record names from `NOTES.md`; the rules are fixed):
- `prompt`: the user-role message that the human typed. Excluded: environment/context messages, AGENTS.md or instruction injections, compaction summaries, anything whose origin the log does not show as the user's input. `chars` = char count of the text; `(chars, relevance_pm, novelty_bytes)` from `ctx.prompt_features(root, text, last_output, last_error, &tail)` where `last_output` is the preceding assistant text, `last_error` the preceding failed command output, and `tail` the session's `ContextTail` (8 KiB, same as the Claude adapter). Text is dropped after.
- `interrupt`: the record marking a turn aborted by the user.
- `tool_decision`: a command or patch approval request answered by the user; `approve` or `deny` per the answer. Commands the sandbox auto-allowed are not decisions.
- `agent_busy`: span from turn start to turn end/abort; a turn with no end yet is not emitted and holds the watermark (the poll wrapper uses `ParseReport.last_ts` and the open-turn start).
- `agent_output`: `chars` of assistant message text per message.
- Agent `test_run`: not emitted (command capture is plan 04's). Agent file writes (patch/apply records): push `(file_hash, ts)` into `Parsed.agent_write_hashes` where `file_hash` is the `heartbeat::file_hash(project_id, rel_path)` function defined in Task 16 (`rel_path` relative to the root from `ctx.project_for_dir`, `/` separators); no event. The source hands them to the runner through `take_agent_writes()`.
- Ids: native record id when the log has one, else `codex:<session>:<line index>:<kind>`.

- [ ] **Step 1: Write the failing tests** in `tests/golden.rs`: `codex_v<version>_golden` (`golden::<Codex>("tests/fixtures/codex/<version>")`, expected written with `TALLY_BLESS=1` then reviewed by hand against `NOTES.md`: the number of prompts equals the number of human prompts you typed in Task 4), `codex_never_emits_injected_context_as_prompt` (fixture contains an environment-context user message; assert prompt count excludes it), `codex_no_text_leak` (`assert_no_text_leak` with words from the unscrubbed prompts that you keep in a local-only list in the test: use the scrub placeholders only, i.e. assert no `"x"` runs longer than 0 appear; simplest is asserting the serialized events contain none of the strings `"text"`, `"cmd"`, `"path"` keys), `codex_format_drift` (drift fixture: no panic, `errors == 2`, `skipped >= 1`, remaining events identical to the clean fixture's events for the intact lines), `codex_reread_gives_same_ids` (parse twice, ids equal).
- [ ] **Step 2:** `cargo test -p tally-daemon --features test-kit codex`. Expected: FAIL.
- [ ] **Step 3:** Implement the parser to the mapping rules. Skip with a `skipped` count anything not recognized.
- [ ] **Step 4:** Run. Expected: PASS, no warnings. Read the blessed `expected.json` once against the real session you ran.
- [ ] **Step 5:** Commit `feat(daemon): codex session parser`.

---

### Task 6: Codex source (discovery, cursor, watermark, config)

**Files:**
- Modify: `crates/tally-daemon/src/adapters/codex.rs`, config loader from plan 03 (add table `[codex]`), `crates/tally-daemon/src/run.rs` (register the source in `sources(cfg, home, store)`)

**Interfaces:** `pub struct CodexSource { root: PathBuf, ctx: RealCtx }`; `CodexSource::new(cfg: CodexConfig)`. Config: `[codex] enabled = true` (default true when the log directory exists), `home = "<path>"` (default `$CODEX_HOME` else `~/.codex`). `poll`: list `<home>/sessions/**/rollout-*.jsonl` modified in the last 7 days (older files are ignored; live capture only), for each call `common::read_new` then `Codex::parse`, commit the cursor after the events are returned, set `self.health` (drained by `take_health`) and `self.agent_writes` (drained by `take_agent_writes`). Watermark: `common::file_watermark(now_ms, open turn starts over all files)`, i.e. `min(now_ms - WRITE_SLACK_MS, open turn start - 1)`. State key per file: `cursor:<normalized path>`.

- [ ] **Step 1: Write the failing tests** in `codex.rs` (temp dir as `home`): `poll_reads_new_lines_only` (write 2 lines, poll, append 1 line, poll: second poll returns events only for the new line), `poll_ignores_files_older_than_seven_days` (set mtime back with `filetime` dev-dependency or `std::fs::File::set_modified`), `open_turn_holds_the_watermark_below_its_start`, `closed_turns_give_now_minus_write_slack` (watermark `== now_ms - WRITE_SLACK_MS`, not the newest event ts), `missing_home_is_no_logs_not_an_error` (poll returns empty, `take_health()` gives `files_found = 0`), `patch_records_come_out_of_take_agent_writes` (drained: second call empty), `unknown_config_key_is_an_error`.
- [ ] **Step 2:** Run. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run. Expected: PASS. Then run the real thing once: `tally run --once --home <tmp>` against your real `~/.codex`, then `tally doctor`. Expected: `codex ok`, prompt count matches Task 4.
- [ ] **Step 5:** Commit `feat(daemon): codex source`.

---

### Task 7: Aider spike (fixture and notes)

**Files:**
- Create: `crates/tally-daemon/tests/fixtures/aider/<version>/{.aider.chat.history.md,.aider.input.history,NOTES.md}`, `docs/adapters/aider.md`

State on the plan author's machine: Aider is not installed (`where aider` finds nothing), so no sample exists. Needed from the executor or maintainer: `pip install aider-chat` (or `pipx`) and an API key for any model Aider supports (a free or local one is enough; the adapter only reads logs).

Expected from memory (UNVERIFIED, confirm or correct in `NOTES.md`): Aider writes into the project directory, not the user's home: `.aider.chat.history.md` (Markdown transcript; a `# aider chat started at <local time>` heading per launch, user messages as lines starting `#### `, tool and model output as `> ` blockquote lines or plain text) and `.aider.input.history` (prompt_toolkit history: `# <timestamp>` line then `+<line>` lines per input). Per-message timestamps are probably absent from the chat history, so per-prompt times come from the input history. Paths can be changed by `--chat-history-file`, `--input-history-file` or the `.aider.conf.yml`; `--llm-history-file` logs raw model traffic (not used). No per-user central directory, so discovery must be per tracked project.

- [ ] **Step 1:** Install Aider, run a scratch session in a scratch git repo: two prompts, one `/run` shell command approved, one declined, one Ctrl-C during a response, `/commit`-style auto-commit, exit.
- [ ] **Step 2:** Record in `NOTES.md` (each `OBSERVED` or `UNVERIFIED`): both files' exact names and where they appear, the timestamp format and timezone of each, how a human message is distinguished from a slash command (`/add`, `/run` are commands, not prompts: Aider treats them as no prompt), how shell-command confirmations and answers appear (if at all), whether an interrupt is recorded (likely not), whether the model's output is in the history (needed for `agent_output`), the Aider version. Decide and write down: which signals are available for `prompt`, `agent_output`, `agent_busy` (probably none: no turn start/end timestamps), `tool_decision` (probably none), `interrupt` (probably none).
- [ ] **Step 3:** Scrub (same rules as Task 4 Step 3) and commit the two fixture files plus a `drift/` variant (heading text changed, a mangled line).
- [ ] **Step 4:** Commit `test(daemon): aider fixtures and format notes`. If Aider cannot be installed or run, commit only `NOTES.md` marked `UNVERIFIED` and stop Tasks 7 and 8.

**Honest expectation:** Aider's logs likely support only `prompt` (from input history timestamps) and `agent_output` (chars, with timestamps interpolated from the preceding prompt: do not do this; skip `agent_output` unless a timestamp exists). Without `agent_busy`, the scorer cannot credit time between a prompt and the next action beyond the generic interval rules; that is acceptable and must be stated in `docs/adapters/aider.md`.

---

### Task 8: Aider parser and source

**Files:**
- Create: `crates/tally-daemon/src/adapters/aider.rs`
- Modify: `crates/tally-daemon/src/adapters/mod.rs`, config loader (`[aider]`), `run.rs` (register in `sources()`)

**Interfaces:** `pub struct Aider; impl Adapter for Aider { const ID = "aider"; }`; `parse` takes the text of `.aider.input.history` (the file with timestamps); the project is the directory containing the file (`sources(cfg, home, store)` reads the tracked project roots from the store once, when it builds the source, and passes them to `AiderSource::new`; the source itself never reads the store, and no filesystem scan of the machine happens; a project added while `tally run` is running is picked up on the next start). Config: `[aider] enabled = true`, `extra_history_files = ["<path>", ...]` for relocated files.

**Mapping:** one `prompt` per history entry that is not a slash command (`/...`) and not empty; `chars`, features from `ctx.prompt_features(root, text, "", "", &tail)` (no assistant context available unless `NOTES.md` shows the chat history is parseable, in which case use the preceding output tail from it, matched by order, never by time); id `aider:<session-start-ms>:<entry index>:prompt`. Nothing else is emitted unless Task 7 found a timestamped signal.

- [ ] **Step 1: Write the failing tests:** `aider_v<version>_golden`, `aider_slash_commands_are_not_prompts`, `aider_no_text_leak`, `aider_format_drift`, `aider_reread_gives_same_ids`, `aider_poll_reads_only_tracked_projects` (a history file in an untracked directory is never opened).
- [ ] **Step 2:** `cargo test -p tally-daemon --features test-kit aider`. Expected: FAIL.
- [ ] **Step 3:** Implement parser and `AiderSource` (uses `common::read_new`; watermark = `common::file_watermark(now_ms, [])` = `now_ms - WRITE_SLACK_MS`: prompts are point events with no open span, but a history entry can be flushed after its timestamp).
- [ ] **Step 4:** Run. Expected: PASS.
- [ ] **Step 5:** Commit `feat(daemon): aider adapter`.

---

### Task 9: Gemini CLI spike

**Files:**
- Create: `crates/tally-daemon/tests/fixtures/gemini-cli/<version>/{session.json,logs.json,NOTES.md}`, `docs/adapters/gemini-cli.md`

State on the plan author's machine: not installed (`where gemini` finds nothing). Needed: `npm i -g @google/gemini-cli` and a Google login or API key (free tier works).

Expected from memory (UNVERIFIED): state under `~/.gemini/` (Windows `%USERPROFILE%\.gemini`); `~/.gemini/tmp/<project-hash>/logs.json` is a JSON array of user prompt records (`sessionId`, `messageId`, `type: "user"`, `message`, `timestamp` ISO-8601); `~/.gemini/tmp/<project-hash>/chats/session-*.json` hold whole conversations (`messages` array with `type` of `user` or `gemini`, `timestamp`, `content`, tool calls). `<project-hash>` is likely the SHA-256 of the project path, which gives a way to map a hash dir to a tracked project without reading text; verify by hashing your scratch repo path in both normalized and raw forms. Files are probably rewritten whole, not appended. Gemini CLI also supports OpenTelemetry output (`telemetry` settings in `settings.json`) which could be a better source: record in the notes whether it is practical, but do not build on it in this plan.

- [ ] **Step 1:** Install and run a scratch session (two prompts, one tool call approved, one denied, one Esc cancel, exit).
- [ ] **Step 2:** Fill `NOTES.md` as in Task 7 Step 2 (paths per OS from docs: `GEMINI_CLI_HOME` or similar override if any; how `/` commands, tool confirmations and cancellation appear; the version field).
- [ ] **Step 3:** Scrub and commit fixtures plus a `drift/` variant (key renamed, array element of the wrong type).
- [ ] **Step 4:** Commit `test(daemon): gemini cli fixtures and format notes`. If it cannot be run, commit `NOTES.md` only and stop Tasks 9 and 10.

---

### Task 10: Gemini CLI parser and source

**Files:**
- Create: `crates/tally-daemon/src/adapters/gemini.rs`
- Modify: `adapters/mod.rs`, config (`[gemini_cli] enabled, home`), `run.rs` (register in `sources()`)

**Interfaces:** `pub struct Gemini; impl Adapter for Gemini { const ID = "gemini-cli"; }`. Because files are rewritten whole, `GeminiSource` does not use the byte cursor: it re-reads a file when `(mtime, size)` changed (stored as `stat:<path>` in `SourceState`) and relies on stable ids for dedup (`gemini-cli:<messageId>:prompt`, or `<sessionId>:<index>` when absent). Project mapping: hash dir to project by hashing each tracked project root as `NOTES.md` determined (roots passed in by `sources(cfg, home, store)` at build time, as for Aider); unmapped hash dirs give `project: None`. Watermark: `common::file_watermark(now_ms, open turn starts)`.

**Mapping:** `prompt` from `type: "user"` messages that are human input (not tool responses echoed with a user role); `agent_output` from model messages (`chars`); `tool_decision` and `interrupt` only if the log records them (Task 9 notes), otherwise omitted and listed in the doctor's `Unsupported` signals.

- [ ] **Step 1: Write the failing tests:** `gemini_v<version>_golden`, `gemini_tool_responses_are_not_prompts`, `gemini_no_text_leak`, `gemini_format_drift`, `gemini_reread_after_rewrite_gives_same_ids` (parse the file, append a message, parse again: old ids unchanged), `gemini_unmapped_hash_dir_gives_no_project`.
- [ ] **Step 2:** Run `cargo test -p tally-daemon --features test-kit gemini`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run. Expected: PASS.
- [ ] **Step 5:** Commit `feat(daemon): gemini cli adapter`.

---

### Task 11: GitHub Copilot spike (VS Code chat and Copilot CLI)

**Files:**
- Create: `crates/tally-daemon/tests/fixtures/copilot/<version>/{chat-session.json,NOTES.md}` (and `cli/` if Copilot CLI is also supported), `docs/adapters/copilot.md`

State on the plan author's machine: VS Code is installed (`code` on PATH), so a real chat session can be captured if the Copilot extension and a GitHub login are present; this was not checked. Copilot CLI is not checked.

Expected from memory (UNVERIFIED): VS Code stores chat sessions per workspace as JSON under `<user data>/User/workspaceStorage/<workspace-hash>/chatSessions/<session-id>.json` where `<user data>` is `%APPDATA%\Code` (Windows), `~/Library/Application Support/Code` (macOS), `~/.config/Code` (Linux); `Code - Insiders` and VSCodium have sibling directories. `workspaceStorage/<hash>/workspace.json` holds the workspace folder URI, which maps a hash dir to a project root. A session file has `requests[]`, each with the user message (`message.text`), a `timestamp`, and a response with parts and timing. The newer VS Code releases have changed this storage more than once (possibly to an append-only log), which is exactly the format risk in PITCH.md. Copilot CLI (`@github/copilot`) keeps session state under `~/.copilot/` (UNVERIFIED layout, possibly `session-state/<id>/events.jsonl`).

- [ ] **Step 1:** In VS Code with Copilot Chat: open a scratch git repo, run two chat prompts in agent mode, accept one tool call, reject one, cancel one response (Stop button), close the window. If Copilot CLI is available, run a short session too.
- [ ] **Step 2:** Locate the files (`Get-ChildItem -Recurse "$env:APPDATA\Code\User\workspaceStorage" -Filter *.json | Sort LastWriteTime -Desc | Select -First 10`). Fill `NOTES.md`: exact path and file name per OS, the VS Code and Copilot extension versions, how a human request is distinguished from an agent-initiated one, how accept/reject of a tool call and Stop appear (if they do), whether per-request timestamps and response durations exist (needed for `agent_busy`), whether files are rewritten or appended, and the `workspace.json` mapping.
- [ ] **Step 3:** Scrub and commit fixtures plus `drift/` (renamed `requests` key, one request missing its timestamp).
- [ ] **Step 4:** Commit `test(daemon): copilot fixtures and format notes`. If no Copilot session can be produced, commit `NOTES.md` marked `UNVERIFIED` and stop Tasks 11 and 12. Needed from the maintainer in that case: a machine with the Copilot subscription, or a sample session file contributed by a user.

---

### Task 12: Copilot parser and source

> As built (PR #12): the adapter targets the Copilot CLI log, `~/.copilot/session-state/<id>/events.jsonl` (override with `COPILOT_HOME`), not VS Code `chatSessions` JSON; see `docs/adapters/copilot.md`. Interrupt, tool_decision and agent writes are never emitted.

**Files:**
- Create: `crates/tally-daemon/src/adapters/copilot.rs`
- Modify: `adapters/mod.rs`, config (`[copilot] enabled, user_data_dirs = [...]`, default list per OS for Code, Code - Insiders), `run.rs` (register in `sources()`)

**Interfaces:** `pub struct Copilot; impl Adapter for Copilot { const ID = "copilot"; }`. `CopilotSource` lists `<user data>/User/workspaceStorage/*/chatSessions/*.json`, maps each hash dir to a project root through its `workspace.json` (decoding the `file:///` URI per OS; a remote or virtual workspace maps to `None`), re-reads a file when `(mtime, size)` changed (or uses the byte cursor if Task 11 found it append-only), dedups by stable ids `copilot:<session>:<request-id-or-index>:<kind>`. Watermark: `common::file_watermark(now_ms, open request starts)`.

**Mapping:** `prompt` per human request (chat input the user typed; not follow-ups generated by the agent loop); `agent_output` from response text `chars`; `agent_busy` from request timestamp to response completion when both exist; `interrupt` only if cancellation is recorded; `tool_decision` only if accept/reject is recorded. Anything the log does not show is not emitted and appears in the doctor as an unsupported signal (see Honesty table below).

- [ ] **Step 1: Write the failing tests:** `copilot_v<version>_golden`, `copilot_agent_followups_are_not_prompts`, `copilot_no_text_leak`, `copilot_format_drift`, `copilot_workspace_hash_maps_to_project` (temp `workspace.json` with a file URI; also a URI with a space and a Windows drive `file:///c%3A/...`), `copilot_remote_workspace_gives_no_project`.
- [ ] **Step 2:** Run `cargo test -p tally-daemon --features test-kit copilot`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run. Expected: PASS.
- [ ] **Step 5:** Commit `feat(daemon): copilot adapter`.

---

### Task 13: Cursor spike (hooks)

**Files:**
- Create: `crates/tally-daemon/tests/fixtures/cursor/<version>/{beforeSubmitPrompt.json,stop.json,beforeShellExecution.json,afterShellExecution.json,afterFileEdit.json,NOTES.md}`, `docs/adapters/cursor.md`

State: Cursor is not installed on the plan author's machine. Cursor's chat history lives in its own SQLite state database, which is undocumented and changes between releases; this plan does not read it. The supported route is Cursor's hooks feature (introduced in Cursor 1.7, UNVERIFIED which versions have it): a `hooks.json` that runs a user-configured command per agent event, passing a JSON object on stdin.

Expected from memory (UNVERIFIED, from Cursor's hooks documentation at https://docs.cursor.com): `hooks.json` at `~/.cursor/hooks.json` (user) or `<project>/.cursor/hooks.json`, shape `{ "version": 1, "hooks": { "<event>": [ { "command": "<shell command>" } ] } }`; events include `beforeSubmitPrompt`, `beforeShellExecution`, `afterShellExecution`, `beforeMCPExecution`, `afterFileEdit`, `stop`; stdin JSON includes `conversation_id`, `generation_id`, `workspace_roots`, and event fields (the prompt text for `beforeSubmitPrompt`, the command for shell hooks, the file path and edits for `afterFileEdit`); some hooks expect a JSON reply on stdout (for example `{"continue": true}` or a permission decision) and a non-zero exit may block the action.

- [ ] **Step 1:** Needed: Cursor installed, hooks enabled. Create a project `.cursor/hooks.json` whose commands write stdin to files (`node -e "require('fs').appendFileSync('C:/tmp/h.json', require('fs').readFileSync(0)+'\n')"`), run two prompts with one shell command approved and one rejected, one stop.
- [ ] **Step 2:** Fill `NOTES.md`: the exact `hooks.json` schema and file locations per OS, every event name and its stdin fields (names only; no values), what stdout reply each event requires so the hook never blocks or alters Cursor, whether there is an event for the user rejecting a command, whether a user-typed prompt is distinguishable from an agent-generated one, the Cursor version.
- [ ] **Step 3:** Scrub captured stdin objects into the fixture files (prompt text to `x` runs of equal length, paths to `C:/tallyproj/...`, command to `cmd1`) and commit.
- [ ] **Step 4:** Commit `test(daemon): cursor hook fixtures and notes`. If Cursor is unavailable, commit `NOTES.md` marked `UNVERIFIED` and stop Tasks 13 to 15; Cursor cannot be done without a machine with Cursor and a sample of its hook payloads.

---

### Task 14: `tally hook` entry point and install

**Files:**
- Create: `crates/tally-daemon/src/hook.rs`
- Modify: `crates/tally-daemon/src/main.rs` (add two variants to plan 04's `enum HookCmd`, README ruling 12: `Cursor { event: String }` for `tally hook cursor <event>`, and `Install { harness: InstallTarget /*value enum, only `cursor`*/, #[arg(long)] project: Option<PathBuf>, #[arg(long)] write: bool }` for `tally hook install cursor`; `HookCmd::TestRun` stays plan 04's), `lib.rs`

**Interfaces (`hook.rs`):**

```rust
pub fn run_cursor_hook(event: &str, stdin: &str, now_ms: i64, spool_dir: &Path, ctx: &dyn ParseCtx) -> String; // returns the stdout reply
pub fn install_cursor(hooks_json: &str, tally_exe: &str) -> Result<String, HookError>; // returns merged JSON
```

Behavior of `run_cursor_hook` (the hook runs inside Cursor's loop, so it must be fast and must never fail or alter the user's flow):
1. Parse the stdin JSON leniently. Any failure, panic, or timeout: write nothing, return the harmless reply (`{"continue":true}` or the reply `NOTES.md` states for that event), exit code 0. A `catch_unwind` wraps the body; the CLI wrapper sets a 400 ms budget for feature computation after which `relevance_pm = 0, novelty_bytes = 0` are written.
2. Reduce to derived numbers, then append one NDJSON line to `<spool_dir>/cursor.ndjson`: `{"ev":"prompt","ts":<now_ms>,"session":"<conversation_id>","cwd":"<workspace root>","chars":N,"relevance_pm":N,"novelty_bytes":N,"id":"<generation_id>"}`; `shell_start`/`shell_end` carry `cmd_hash` and `exit_code` only; `file_edit` carries `file_hash` (`heartbeat::file_hash`) and `lines_changed` (the `cursor` source reports it as an agent write through `take_agent_writes`, Task 15); `stop` carries the session and `ts`. The spool line holds the workspace root path (needed to resolve the project; local file, never uploaded, never an `Event` field). Spool lines are one `write` call under 4 KiB with `O_APPEND` semantics so concurrent hooks do not interleave.
3. Timestamp is the hook's `now_ms`, taken at entry (hooks fire at the moment of the action).
4. The user's own prompt text is never written anywhere.

`install_cursor(existing, exe)`: if `existing` is empty, produce a new file; otherwise parse and add `{"command":"\"<exe>\" hook cursor <event>"}` to each of `beforeSubmitPrompt`, `beforeShellExecution`, `afterShellExecution`, `afterFileEdit`, `stop` unless an identical command is already present; keep all other entries and unknown keys; never overwrite a malformed file (return an error telling the user to fix or remove it). CLI: `tally hook install cursor [--project <path>]` prints the merged JSON and the target path and writes it only with `--write`.

- [ ] **Step 1: Write the failing tests** in `hook.rs`: `prompt_hook_writes_derived_numbers_only` (stdin with a unique word; spool line parsed as JSON; the word and the full stdin are absent from the file), `garbage_stdin_gives_harmless_reply_and_no_spool_line`, `hook_reply_is_continue_true_for_prompt_event`, `concurrent_hooks_do_not_interleave_lines` (8 threads, 50 lines each, every line parses), `install_adds_all_events_once` (run twice, same output), `install_keeps_existing_entries_and_unknown_keys`, `install_refuses_malformed_file`.
- [ ] **Step 2:** `cargo test -p tally-daemon hook`. Expected: FAIL.
- [ ] **Step 3:** Implement. Replies are taken from `NOTES.md` (Task 13), not guessed.
- [ ] **Step 4:** Run. Expected: PASS. Manual check with real Cursor if available: `tally hook install cursor --project . --write`, restart Cursor, send a prompt, expect one spool line; kill the `tally` binary path and expect Cursor to carry on normally.
- [ ] **Step 5:** Commit `feat(daemon): tally hook entry point and cursor install`.

---

### Task 15: Cursor spool parser and source

**Files:**
- Create: `crates/tally-daemon/src/adapters/cursor.rs`
- Modify: `adapters/mod.rs`, config (`[cursor] enabled`), `run.rs` (register in `sources()`)

**Interfaces:** `pub struct Cursor; impl Adapter for Cursor { const ID = "cursor"; }`; `parse` reads spool lines and maps: `prompt` (numbers copied), `agent_busy` from a prompt to the next `stop` of the same session (a prompt with no stop yet is an open span and holds the watermark), `agent_output` not available, `tool_decision` only if Task 13 found a user-rejection event (an approved shell command is inferred as `approve` only if `NOTES.md` says the hook fires after user confirmation; otherwise not emitted), agent write hashes from `file_edit` lines (out through `take_agent_writes`). Project from `cwd` via `ctx.project_for_dir`. Ids `cursor:<generation_id>:<kind>`. `CursorSource` reads the spool with `common::read_new`; watermark `common::file_watermark(now_ms, open prompt starts)`.

- [ ] **Step 1: Write the failing tests:** `cursor_v<version>_golden` (spool fixture generated by running Task 14's `run_cursor_hook` over the Task 13 fixtures), `cursor_open_prompt_holds_watermark`, `cursor_format_drift`, `cursor_spool_line_with_unknown_event_is_skipped`.
- [ ] **Step 2:** Run. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run. Expected: PASS.
- [ ] **Step 5:** Commit `feat(daemon): cursor adapter over the hook spool`.

**Honesty table (goes in each `docs/adapters/<harness>.md` and drives `doctor`'s `Unsupported` signals):** fill from the spikes; the expected shape is Codex: prompt, interrupt, tool_decision, agent_busy, agent_output; Aider: prompt only; Gemini CLI: prompt, agent_output (rest if recorded); Copilot: prompt, agent_output, agent_busy (rest if recorded); Cursor: prompt, agent_busy, agent writes, optionally tool_decision, and only after the user installs hooks.

---

### Task 16: Heartbeat protocol spike and request types

**Files:**
- Create: `crates/tally-daemon/src/heartbeat/mod.rs`, `crates/tally-daemon/tests/fixtures/heartbeat/{single.json,bulk.json,wakatime-cli-requests.txt,NOTES.md}`
- Modify: `lib.rs` (`pub mod heartbeat;`), `Cargo.toml` (`serde = { version = "1", features = ["derive"] }`, `base64 = "0.22"`)

Expected from memory of the WakaTime API (UNVERIFIED; this task confirms it and the notes override this text):
- Endpoints under `<api_url>/v1/users/current/`: `POST heartbeats` (one object) and `POST heartbeats.bulk` (JSON array, at most 25 elements). `wakatime-cli` normally sends bulk and replays its offline queue the same way. The `api_url` setting in `~/.wakatime.cfg` is a base such as `https://wakatime.com/api`; the client appends `/v1/users/current/heartbeats.bulk`. Hackatime serves the same routes under `/api/hackatime/v1/`, so accept both prefixes.
- Auth: `Authorization: Basic <base64(api_key)>` (the key itself is base64-encoded, no `user:` prefix), or `?api_key=` in the query string; `Bearer <key>` is accepted by WakaTime for OAuth tokens.
- Heartbeat object fields: `entity` (path, URL or app name), `type` (`file`, `app`, `domain`), `time` (float seconds since epoch), `category` (`coding`, `debugging`, `ai coding`, ...), `project`, `branch`, `language`, `is_write` (bool), `lines` (total lines in the file), `lineno`, `cursorpos`, `line_additions`, `line_deletions`, `ai_line_changes`, `human_line_changes` (newer clients), `user_agent`, `plugin`.
- Responses: single `201` with `{"data": {"id": "...", "entity": ..., "type": ..., "time": ...}}`; bulk `201` (WakaTime) or `202` with `{"responses": [[{"data": {...}}, 201], ...]}`, one pair per input element, in order. `400` for malformed JSON. A client treats non-2xx as a failure and queues offline, so every accepted-and-dropped heartbeat must still answer 201 (Review Focus 3 and 4).

- [ ] **Step 1:** Install any WakaTime editor plugin or `wakatime-cli` (VS Code is on the author's machine: the "WakaTime" extension). Run a tiny listener that prints method, path, headers (redact the key) and body to a file: `python -m http.server` will not do; use a 10-line script in a scratch directory, not in the repo, started with `python -I script.py <port>`. Point `api_url = http://127.0.0.1:<port>/api` and `api_key = test-key` in `~/.wakatime.cfg` (back the file up first; restore after). Edit a file for a minute.
- [ ] **Step 2:** Record in `NOTES.md` the exact paths requested (also any `GET` such as status bar or `users/current` calls and what a 404 does to the plugin), the exact `Authorization` form, whether `wakatime-cli` sends single or bulk, `Content-Type`, `User-Agent`, whether `Host` carries the port, field set observed, and whether a `201` with the response shapes above satisfies the client (watch that it does not resend). Commit scrubbed request bodies (`entity` replaced by `C:/tallyproj/src/main.rs`) as the fixtures.
- [ ] **Step 3: Write the failing tests** in `heartbeat/mod.rs`: `parses_single_and_bulk_fixtures`, `unknown_fields_are_ignored`, `missing_entity_or_time_is_an_error_for_that_heartbeat_only`, `time_float_seconds_to_ms` (`1763800000.123` gives `1763800000123`), `auth_accepts_basic_base64_key_and_query_key_and_bearer`, `auth_rejects_wrong_key_with_401_and_constant_time_compare` (assert the compare function is `subtle`-free: implement with a manual XOR loop over equal-length padding, no early return), `file_hash_is_stable_and_path_free` (below).
- [ ] **Step 4:** Implement: `pub struct Heartbeat { entity: String, kind: Option<String>, time: f64, category: Option<String>, is_write: Option<bool>, lines: Option<u32>, … }` (all optional except `entity` and `time`; `#[serde(rename = "type")]`), `pub fn parse_body(bytes: &[u8]) -> Result<Vec<Result<Heartbeat, String>>, BodyError>` (a bulk array yields per-element results; an object yields one), `pub fn check_auth(header: Option<&str>, query_key: Option<&str>, token: &str) -> bool`, and

```rust
/// Never stores or returns the path. Hash of "<project_id>\0<path relative to the git root, '/' separators>".
pub fn file_hash(project_id: &str, rel_path: &str) -> String;
```

  Task 5, 12 and 15 adapters and the Claude Code source (Task 19) use the same function for agent writes (the repository-relative path is available to every harness since they know cwd and the edited path).
- [ ] **Step 5:** Run `cargo test -p tally-daemon heartbeat`. Expected: PASS. Commit `feat(daemon): wakatime heartbeat types, auth, file hash`.

---

### Task 17: Heartbeat HTTP server (localhost only, limits)

**Files:**
- Create: `crates/tally-daemon/src/heartbeat/server.rs`, `crates/tally-daemon/tests/heartbeat.rs`
- Modify: `heartbeat/mod.rs`, `Cargo.toml` (`tiny_http = "0.12"`), config (`[heartbeat] enabled = false, port = 25930, token_file = "<home>/heartbeat.token"`)

**Interfaces:**

```rust
pub struct ServerConfig { pub port: u16 /*0 = ephemeral, for tests*/, pub token: String, pub max_body: usize /*default 262_144*/, pub max_bulk: usize /*default 25*/ }
pub struct Running { pub addr: std::net::SocketAddr, /* join handle, shutdown flag */ }
pub fn start(cfg: ServerConfig, sink: Arc<Mutex<Vec<Received>>>) -> std::io::Result<Running>;
pub struct Received { pub hb: Heartbeat, pub received_ms: i64 }
pub fn host_ok(host: Option<&str>) -> bool;   // 127.0.0.1, localhost, [::1], with optional :port
```

Rules: bind `("127.0.0.1", port)` only; `Host` missing or not `host_ok` gives `403` (DNS rebinding); wrong or missing key gives `401` with no body detail (the token is a random 32-byte hex string created on first start with `0600` permissions where supported and stored in `token_file`; `tally heartbeat-config` prints the `~/.wakatime.cfg` lines for the user); methods other than `POST` on the two routes give `405`; routes are `/api/v1/users/current/heartbeats`, `/api/v1/users/current/heartbeats.bulk` and the `/api/hackatime/v1/...` equivalents; `Content-Length` over `max_body` gives `413` without reading the body; a body that reads past `max_body` gives `413`; a bulk array longer than `max_bulk` gives `400`; unknown paths give `404`; at most 8 connections are served at once (others wait; tiny_http single thread plus a small worker pool is enough). Responses use the shapes in Task 16 notes (one `201` per heartbeat, malformed elements answered `400` inside the bulk pair). The server pushes parsed heartbeats into `sink` and does nothing else (no file I/O, no store access), so a slow disk cannot stall an editor.

- [ ] **Step 1: Write the failing tests** in `tests/heartbeat.rs` (raw `std::net::TcpStream` HTTP/1.1 requests against `start(port: 0)`): `binds_loopback_only` (`addr.ip().is_loopback()` and `== 127.0.0.1`), `foreign_host_header_is_403`, `localhost_host_with_port_is_accepted`, `missing_or_wrong_key_is_401`, `single_heartbeat_gets_201_data`, `bulk_gets_201_responses_in_order_with_per_item_status` (3 items, middle one malformed gives `[..,201],[..,400],[..,201]`), `oversized_content_length_is_413`, `bulk_over_25_is_400`, `get_is_405`, `unknown_path_is_404`, `sink_receives_only_valid_heartbeats`.
- [ ] **Step 2:** Run `cargo test -p tally-daemon --test heartbeat`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run. Expected: PASS.
- [ ] **Step 5:** Commit `feat(daemon): localhost wakatime heartbeat server`.

---

### Task 18: Heartbeat filter and `user_edit` mapping (`HeartbeatSource`)

**Files:**
- Modify: `crates/tally-daemon/src/heartbeat/mod.rs`, `crates/tally-daemon/src/store.rs` (table `agent_writes(file_hash TEXT, ts INTEGER, PRIMARY KEY(file_hash, ts))` with `put_agent_writes(&[(String,i64)])` (`INSERT OR IGNORE`, so a re-emitted batch is harmless) and `agent_writes_since(ts) -> Vec<(String,i64)>`; only `run.rs` calls them), `source.rs` (default method `fn see_agent_writes(&mut self, _writes: &[(String, i64)]) {}`), `run.rs` (plan 03; see below), `run.rs::sources()` (build `HeartbeatSource`, which starts the server, when `[heartbeat] enabled`)

**Runner (`run.rs`), the only place agent writes touch the store:** in `run_once`, right after each source's `poll` returns `Ok`, `let w = source.take_agent_writes()`; `store.put_agent_writes(&w)` before `append_events` and `set_watermark` (same position as plan 04's `take_coverage`); collect `w` for the tick. After all sources have polled, call `see_agent_writes(&tick_writes)` on every source. At start-up `sources(cfg, home, store)` seeds `HeartbeatSource` with `store.agent_writes_since(now_ms - 10 * 60_000)` so a restart does not forget recent agent writes. `HeartbeatSource` keeps them in memory, pruned to the last 10 minutes; it never reads the store.

**Constants (`heartbeat/mod.rs`):** `AGENT_WRITE_WINDOW_MS = 10_000`, `HEARTBEAT_HOLD_MS = 15_000`, `HEARTBEAT_SLACK_MS = 60_000`, `EDIT_BUCKET_MS = 120_000`.

**Interfaces:**

```rust
pub fn map_heartbeats(
    batch: &[Received],
    project_of: &dyn Fn(&Path) -> Option<(PathBuf, String)>, // production: project::project_for_dir, cached per dir
    agent_writes: &dyn Fn(&str, i64, i64) -> bool,   // (file_hash, from_ms, to_ms), over the in-memory list
    floor: i64,                               // the watermark this poll returns; heartbeats at or below it are late
    prev_lines: &mut BTreeMap<String, u32>,   // file_hash -> last seen `lines`
) -> Mapped;
pub struct Mapped { pub events: Vec<Event>, pub stats: Stats }
pub struct Stats { pub accepted: usize, pub dropped_agent: usize, pub outside_project: usize, pub late_dropped: usize, pub not_a_file: usize }
```

Rules, in order, per heartbeat:
1. `type != "file"` (apps, domains) or `is_write` irrelevant: not a file edit, `not_a_file += 1`. (Only file heartbeats are edits; reading a file also sends heartbeats, so require either `is_write == true` or a change in `lines` since the previous heartbeat for that file; plain cursor movement is not an edit.)
2. Normalize `entity` (`project::normalize_path`), `project_of(parent dir)` (the same git-root mapping the Claude Code adapter uses, so a nested repository maps to the inner root). None: `outside_project += 1`, stop; the path is not retained.
3. `rel` = path under the root (prefix strip, path-component aware, case-insensitive on Windows and macOS); `h = file_hash(project_id, rel)`; the path string is dropped here.
4. Agent filter: if `agent_writes(h, ts - AGENT_WRITE_WINDOW_MS, ts)` then `dropped_agent += 1`, stop. (The window is only backward from the heartbeat: the heartbeat is the editor reacting to a write, never the cause. Agent writes seen later in logs are still matched because the hold delays the decision.)
5. `ts <= floor`: `late_dropped += 1`, stop. (`floor` is this source's watermark and the store's `finalized_through` never exceeds the minimum source watermark, so no emitted heartbeat can be a `LateEvent`; the source needs no store read. Exception: the very first poll ever, before this source has a watermark row; at worst that one batch is dropped as late, which the runner already tolerates.)
6. One event per `(h, ts / EDIT_BUCKET_MS)`: `Kind::UserEdit { file_hash: h, lines_changed }`, `lines_changed` = `|lines - prev|` when both known and nonzero, else 1; id `heartbeat:<h>:<bucket>:user_edit`; `source = "heartbeat"`, `project = Some(id)`, `session = None`. The earliest heartbeat of the bucket decides `ts`; sort the batch by `time` first so arrival order does not matter.

`HeartbeatSource` (`id() = "heartbeat"`): `poll` drains `sink` entries with `hb.time_ms <= now_ms - HEARTBEAT_HOLD_MS` (the rest wait; they are in memory only, so a daemon crash loses at most 15 s of edit events, noted as accepted in `docs/adapters/heartbeat.md`), calls `map_heartbeats` with `floor = now_ms - HEARTBEAT_HOLD_MS - HEARTBEAT_SLACK_MS`, and returns `watermark = Some(floor)`. Heartbeats whose `time` is at or below `floor` arrived later than the slack allows and are dropped as `late_dropped`, so the watermark promise holds. `see_agent_writes` appends to the in-memory agent-write list. Stats go into `Health.counters` (`accepted`, `dropped_agent`, `outside_project`, `late_dropped`, `not_a_file`) and out through `take_health()`, so `tally doctor` shows them.

- [ ] **Step 1: Write the failing tests** (pure function, table of heartbeats):
  - `outside_project_is_counted_and_has_no_event` (Review Focus 4; also assert no string from the path is in the serialized events or stats);
  - `agent_write_within_window_discards` (agent write at t, heartbeat at t+9_999 dropped, at t+10_001 kept; same file);
  - `agent_write_for_another_file_does_not_discard`;
  - `heartbeat_before_the_agent_write_is_kept` (user edited, then agent wrote);
  - `out_of_order_bulk_gives_same_events_as_sorted` (shuffle 10 heartbeats, compare events);
  - `bulk_of_25_with_one_late_heartbeat_keeps_the_rest` (the late one at `floor`, others accepted);
  - `heartbeats_in_one_bucket_make_one_event` and `two_buckets_two_events`;
  - `read_only_cursor_movement_is_not_an_edit` (`is_write=false`, same `lines`);
  - `lines_changed_is_the_delta_or_one`;
  - `nested_repo_maps_to_the_inner_root` (two tempdir git repos, one inside the other, real `project_for_dir`);
  - `path_with_backslashes_and_drive_case_matches_on_windows` (`C:\Repo\src\a.rs` against a stub `project_of` returning root `c:/repo` under `cfg(windows)`);
  - `events_roundtrip_through_store_and_score` (append to `Store::open_in_memory`, `finalize` with a watermark, `status.verified_s > 0` for a prompt plus an edit);
  - in `run.rs`: `runner_persists_agent_writes_and_forwards_them` (a `Fake` source whose `take_agent_writes` returns one write: after `run_once`, `store.agent_writes_since(0)` has it, and a second `Fake` recorded it in `see_agent_writes`). `HeartbeatSource::new` takes no `Store`.
- [ ] **Step 2:** Run `cargo test -p tally-daemon heartbeat`. Expected: FAIL.
- [ ] **Step 3:** Implement `map_heartbeats`, the store methods, and `HeartbeatSource`.
- [ ] **Step 4:** Run. Expected: PASS. Add `tests/heartbeat.rs::end_to_end_post_then_poll_emits_one_user_edit` (start server on port 0, POST a bulk, call `poll` with `now_ms` past the hold, assert exactly one `user_edit`, no path text in the DB file: read the SQLite file bytes and assert the path string is absent).
- [ ] **Step 5:** Commit `feat(daemon): heartbeat source maps editor heartbeats to user_edit`.

---

### Task 19: Claude Code agent writes (plan 03 source feeds the heartbeat filter)

Claude Code is the main harness. Its Edit, Write, MultiEdit and NotebookEdit tool calls make the editor send heartbeats for the touched file; without this task they become `user_edit` claims and agent activity moves the badge (README ruling 7).

**Files:**
- Modify: `crates/tally-daemon/src/claude_code/mod.rs` (plan 03). Nothing in `line.rs` or `session.rs` changes.

**Interfaces:**
- Plan 03 already parses `Entry::ToolUse.file_path` (`input.file_path`, or `notebook_path` for `NotebookEdit`) and fills `Session.agent_writes: Vec<(i64, String)>` as `(ts, raw path)` for `Edit`/`Write`/`MultiEdit`/`NotebookEdit`, at `tool_use` time (before the file changes, which the backward `AGENT_WRITE_WINDOW_MS` covers). Do not add `file_path` again and do not change the `Session::feed` signature.
- `mod.rs`: a pure `fn hash_writes(writes: &[(i64, String)], root: &Path, project_id: &str) -> Vec<(String, i64)>`: for each `(ts, path)`, apply `project::normalize_path` first (Claude logs hold absolute OS-separator paths such as `C:\tallyproj\src\a.rs`; plan 03's session test uses relative `src/a.rs` only for brevity), then cut it relative to `root` with `/` separators, drop paths outside `root`, and give `(heartbeat::file_hash(project_id, rel), ts)`. `ClaudeCode::poll`, which in plan 03 only clears each session's `agent_writes`, instead drains them through `hash_writes` (root and `project_id` from the session cwd via `project_for_dir`) and appends the result to a new `ClaudeCode.agent_writes: Vec<(String, i64)>`. The raw path never leaves `poll` and no event is emitted. Warm-up writes are discarded with the warm-up events. `ClaudeCode` implements `fn take_agent_writes(&mut self) -> Vec<(String, i64)> { std::mem::take(&mut self.agent_writes) }`. No store call.

- [ ] **Step 1: Write the failing tests.** Plan 03's session-level `agent_file_edits_are_recorded_for_plan_08` test (raw `(ts, path)`) stays UNCHANGED; this task ADDS the hashed tests. In `mod.rs`: `hash_writes_hashes_root_relative_paths` (root `C:/tallyproj`, project id `"p"`: writes `(1, "C:\\tallyproj\\src\\a.rs")` and `(2, "C:/tallyproj/src/a.rs")` both give `heartbeat::file_hash("p", "src/a.rs")` at their `ts`; `(3, "D:/other/b.rs")` outside the root gives nothing; the output contains no `"src/a.rs"` string). A `poll`-level test is optional and, if written, uses a tempdir git repo as the session cwd like plan 03's `tests/claude_code.rs` (the real `project_for_dir` returns `(root, project_id(root))`, never `"p"`): Edit, Write, MultiEdit, NotebookEdit give four `take_agent_writes()` entries, `Read` none, a second `take_agent_writes()` is empty. Also `tests/claude_code.rs` `claude_edit_suppresses_the_matching_heartbeat` (a fixture session with one `Edit` tool_use at T; `ClaudeCode::poll` then `take_agent_writes` fed to `map_heartbeats` with a heartbeat for the same file at T + 2 s gives `dropped_agent == 1` and no `user_edit`).
- [ ] **Step 2:** `cargo test -p tally-daemon claude_code`. Expected: FAIL.
- [ ] **Step 3:** Implement `hash_writes`, call it from `ClaudeCode::poll`, and add `take_agent_writes`. Leave plan 03's `agent_file_edits_are_recorded_for_plan_08` as it is.
- [ ] **Step 4:** `cargo test -p tally-daemon`. Expected: PASS.
- [ ] **Step 5:** Commit `feat(daemon): claude code edits count as agent writes for the heartbeat filter`.

---

### Task 20: Docs, `heartbeat-config`, CI, and `doctor` coverage

**Files:**
- Create: `docs/adapters/heartbeat.md`, `docs/adapters/README.md` (table: harness, signals, where logs live per OS, status)
- Modify: `crates/tally-daemon/src/main.rs` (`Cmd::HeartbeatConfig` printing the two `.wakatime.cfg` lines `api_url = http://127.0.0.1:<port>/api` and `api_key = <token>`, and the location of the file), `.github/workflows/daemon.yml` (the existing test step becomes `cargo test -p tally-daemon --features test-kit` on the existing three-OS matrix; add `git diff --exit-code crates/tally-daemon/tests/fixtures` after the test run so an accidental `TALLY_BLESS` or CRLF rewrite fails CI), `.gitattributes`

- [ ] **Step 1: Write the failing tests:** `tests/cli.rs::heartbeat_config_prints_loopback_url_and_token_and_creates_token_file` (home is a temp dir; the second run prints the same token), `tests/cli.rs::doctor_lists_heartbeat_counters`.
- [ ] **Step 2:** Run. Expected: FAIL.
- [ ] **Step 3:** Implement; write the docs from the `NOTES.md` files (do not copy unverified claims without the `UNVERIFIED` marker). `docs/adapters/heartbeat.md` must state: editor plugin setup lines, that other local programs cannot post without the token, that only hashes leave the plugin payload, the 15 s hold and its crash-loss ceiling, and that `wakatime.com` is not contacted by Tally (plugins would send to one place only: users who also want wakatime.com keep a second plugin config, out of scope).
- [ ] **Step 4:** `cargo test -p tally-daemon --features test-kit` on Windows, then push the branch and confirm the three-OS CI is green.
- [ ] **Step 5:** Commit `docs(daemon): adapter and heartbeat docs, CI runs fixture tests`.

---

## Done when

- `cargo test -p tally-daemon --features test-kit` passes on Windows, macOS and Linux, every adapter that had a real or documented sample has a committed fixture and golden test, and `git diff --exit-code` over fixtures is clean in CI.
- `tally doctor` prints one line per configured harness and heartbeat counters.
- Codex events from a real session on the author's machine appear in `tally status` after `tally run`.
- An editor plugin pointed at `http://127.0.0.1:<port>/api` produces `user_edit` events and no path appears in the database.
- Each harness whose spike could not be completed is listed in `docs/adapters/README.md` as `unsupported` with what is needed, and no parser exists for it.

## Not in this plan

- Reading Cursor's SQLite chat store, VS Code remote or web chat, JetBrains Copilot, Windsurf, Zed, Continue, OpenCode, Amp (add as new adapters with the Task 1 and 2 kit).
- Gemini CLI OpenTelemetry as a source; counting history from before the daemon first ran; forwarding heartbeats to wakatime.com or Hackatime.
- Multi-user machines, a heartbeat endpoint reachable from other hosts, TLS.
- Plan 12 (installer, auto-detection of which harnesses are installed, service registration) wires these sources into setup; here they are enabled through `config.toml` only.
