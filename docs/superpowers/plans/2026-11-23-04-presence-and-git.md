# Presence, Presence Coverage, Git and Project Matching Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** On Windows the daemon samples the OS idle timer and emits `idle` spans; it records when presence sampling was healthy (coverage) and `finalize` ledgers only claims whose interval lies inside coverage; a git source emits `commit` events (user vs agent); worktrees and clones of one repository roll up under one public name; the verbatim UNC path bug is fixed; user test runs are captured by an explicit wrapper command.

**Architecture:** `presence.rs` holds pure span and coverage logic behind a `Platform` trait, with a Windows implementation (`windows-sys`) and a stub elsewhere. A `Source` gains one default method, `take_coverage`, which the runner persists into a new `coverage` table. `finalize` decides, once per final claim, "ledger" or "skip with reason" and records skips in an append-only `skipped_claims` table. `git.rs` shells out to the `git` command line. `project.rs` gains common-dir/remote matching.

**Tech Stack:** Rust stable; adds `windows-sys` (target-specific, features `Win32_System_SystemInformation`, `Win32_UI_Input_KeyboardAndMouse`). No libgit2.

**Spec:** `docs/spec/scoring.md` §2 (events), §3 (interval, `MAX_INTERVAL`, "missing idle data is presence"), `docs/superpowers/specs/2026-11-22-daemon-roadmap-design.md` (live capture only), `docs/superpowers/plans/README.md` (contracts C1, C4, C5; carried-forward UNC item).

## Global Constraints

- Contract C1 holds: sources never call `finalize`; a source returns watermark `w` only when it will never emit another event with `ts <= w`, and an open `idle` span keeps `w` below its start.
- No scoring change, no ruleset bump: the scorer is untouched. All new behaviour is in the daemon (what is ledgered), so `RULESET_VERSION` stays `2026.11-0`.
- The ledger stays append-only and a prefix of "re-scored claims minus recorded skips". No code path updates or deletes `ledger`, `coverage` rows' past extent, or `skipped_claims`.
- A skip is never silent: it is stored with a reason and shown by `tally status` (`skipped_claims`).
- Events hold only hashes and numbers (C3): commits carry `sha`, `by`, `lines_changed`; no messages, paths, authors, remote URLs. Remote URLs and common-dir paths are stored only in the local `projects` table, never in events or the ledger.
- Time is `i64` ms. Use `ruleset::{IDLE_SAMPLE_MIN_MS, MAX_INTERVAL_MS, FIRST_ACTION_MS, INTERRUPT_FOLLOW_MS}`; do not copy constants.
- Everything compiles and tests pass on Windows, macOS and Linux. The Windows API is touched only inside `#[cfg(windows)]` code that the logic tests never need.
- Git is invoked as `git` with arguments from an array (never through a shell). A missing `git` is a reported, non-fatal condition for the git source.

## Review Focus

1. **Laptop sleep and resume.** After resume, `GetLastInputInfo` reports a huge idle. Expected: the sample gap exceeds `COVERAGE_MAX_GAP_MS`, so coverage is not extended across the gap, any open idle span is closed at the last sample before the gap, and no idle span reaching back into the sleep is emitted. A claim whose interval spans the sleep is skipped with `no_presence_coverage`. Tests in Tasks 2 and 3.
2. **System clock changes.** Backward jump: `now < last_sample` resets coverage and closes the open span at `last_sample`; no negative-length interval, no span with `end_ts < ts`. Forward jump larger than the gap limit behaves as a gap. Tests in Task 2.
3. **Idle span open when the daemon stops.** Expected: the watermark was held below the span start, so on restart the span is closed at the last persisted sample time (gap) or continued (no gap); the event is accepted (not late) and ids are stable so replays are duplicates. Test in Task 2.
4. **Commit in a worktree.** Expected: commits made in any worktree of a repository are seen once (deduplicated by sha) and attributed to the project of the worktree the daemon was told about; two worktrees of one repository registered under one public name roll up. Tests in Tasks 5 and 6.
5. **`finalize` change.** Expected: skips are decided once, only for claims that are final, only when coverage is complete up to the horizon; the ledger prefix check ignores skipped claims; a claim skipped yesterday is not resurrected by coverage appearing later; with presence required and no presence watermark nothing becomes final (so a user who never started the presence source is not silently skipped, they are blocked and told). Tests in Task 3.
6. **Agent commits are not credited to the user.** A `git commit` run by the agent is `by: agent` (no human action), matched to plan 03's agent `commit` event by `by = agent`, project and time window (`sha` ignored). Ordering is guaranteed, not hoped for: a git commit at T is emitted only once the `claude-code` watermark is at least T + 5 s, so a late-arriving marker still matches; the 60 s `GIT_LAG_MS` is only the fallback when claude-code is disabled or has no watermark. Tests in Task 5 (including a marker that arrives late).

## File Structure

```
crates/tally-daemon/
  Cargo.toml                 add [target.'cfg(windows)'.dependencies] windows-sys
  src/store.rs               coverage + skipped_claims tables, presence_required flag, append_ledger takes skips
  src/finalize.rs            coverage guard (Task 3)
  src/status.rs              Status.skipped_claims
  src/source.rs              (plan 03) add default method take_coverage
  src/run.rs                 (plan 03) persist take_coverage output before set_watermark
  src/presence.rs            Platform trait, Tracker (spans + coverage), PresenceSource   (Task 2, 4)
  src/presence_win.rs        #[cfg(windows)] WindowsPlatform; stub elsewhere                (Task 4)
  src/git.rs                 GitSource: commits via the git CLI                              (Task 5)
  src/project.rs             UNC fix, repo_identity, register_git_project matching          (Task 6)
  src/main.rs                `tally hook` subcommand group with `test-run -- <cmd>`           (Task 7)
  tests/cli.rs               hook test
```

---

### Task 1: Store support for coverage and recorded skips

**Files:** Modify `crates/tally-daemon/src/store.rs`, `crates/tally-daemon/src/status.rs`.

**Interfaces:**
- Schema additions (`CREATE TABLE IF NOT EXISTS`): `coverage(start_ts INTEGER NOT NULL, end_ts INTEGER NOT NULL)`; `skipped_claims(action_id TEXT PRIMARY KEY, ts INTEGER NOT NULL, reason TEXT NOT NULL)` with `BEFORE UPDATE`/`BEFORE DELETE` abort triggers like `ledger`; `meta` row `presence_required` (0/1, absent = 0).
- `pub struct Span { pub start: i64, pub end: i64 }` is not reused from scoring (private there); define `pub type Interval = (i64, i64);` in `store.rs`.
- `extend_coverage(&mut self, spans: &[Interval]) -> Result<(), StoreError>`: one transaction; for each `(s, e)` with `s <= e`: if it overlaps or touches (`s <= stored.end`) the stored interval with the greatest `end_ts`, set that row's `end_ts` to `max(end, e)`; otherwise insert. Rows only grow at the end or are added; no code shrinks one. (Coverage is not a ledger; it is a monotone fact table, and the guard decides each claim exactly once.)
- `coverage(&self) -> Result<Vec<Interval>, StoreError>` sorted by `start_ts`.
- `set_presence_required(&mut self, on: bool)`, `presence_required(&self) -> Result<bool, StoreError>`.
- `skipped(&self) -> Result<Vec<(String, i64, String)>, StoreError>` (action id, ts, reason) in ts order.
- `append_ledger(&mut self, entries: &[LedgerEntry], finalized_through: i64, scored_events: usize, skips: &[(String, i64, String)]) -> Result<(), StoreError>` (final form; see "Assumes"): same transaction inserts the skips (`INSERT OR IGNORE`). All existing callers and tests pass `&[]`.
- `Status.skipped_claims: usize` and a `skipped_claims: <n>` line in `tally status` (after `unverified_s`; update `tests/cli.rs` expectations that list the lines).

- [ ] **Step 1: Write failing tests** in `store.rs`:

```rust
#[test] fn coverage_merges_touching_intervals_and_keeps_gaps() {
    let mut s = Store::open_in_memory().unwrap();
    s.extend_coverage(&[(0, 10), (10, 20), (30, 40), (35, 50)]).unwrap();
    assert_eq!(s.coverage().unwrap(), vec![(0, 20), (30, 50)]);
    s.extend_coverage(&[(5, 8), (50, 60)]).unwrap();   // (5,8) is inside (0,20)? only the last row is extended
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
```

(Only the last stored interval can be extended, so `(5, 8)` is ignored because it lies at or before `(0, 20)` and not after the last row; implement "last row only" and keep the assertion above. Coverage is appended in time order by one tracker.)

- [ ] **Step 2:** `cargo test -p tally-daemon store`. Expected: FAIL.
- [ ] **Step 3:** Implement. `extend_coverage` reads the row with `MAX(end_ts)`; if `s <= last.end` it sets `end_ts = MAX(end_ts, e)` on that row, else inserts.
- [ ] **Step 4:** `cargo test -p tally-daemon`. Expected: PASS, no warnings.
- [ ] **Step 5:** Commit: `feat(daemon): coverage and skipped-claim tables`.

---

### Task 2: Presence tracker (pure span and coverage logic)

**Files:** Create `crates/tally-daemon/src/presence.rs`; modify `lib.rs` (`pub mod presence;`).

**Interfaces:**
- Consumes: `ruleset::IDLE_SAMPLE_MIN_MS`; `store::Interval`; plan 03's `SourceState` (`get`/`set`).
- Produces:
  - `pub const COVERAGE_MAX_GAP_MS: i64 = 60_000;` (two samples further apart than this are a gap: sleep, daemon stopped, stalled poll. Poll interval from plan 03 must be well below it; the default there is 15 s (`[run] interval_secs`), so one or two missed polls (30 s, 45 s) still keep coverage and a third breaks it. Test sample times below are arbitrary inputs, not the cadence.)
  - `pub struct IdleSpan { pub start: i64, pub end: i64 }`
  - `pub struct Step { pub closed: Vec<IdleSpan>, pub coverage: Vec<Interval>, pub watermark: i64 }`
  - `#[derive(Default)] pub struct Tracker { last_sample: Option<i64>, open: Option<i64>, wm: i64 }` with `Tracker::load(state: &dyn SourceState) -> Tracker`, `save(&self, state: &mut dyn SourceState)` (keys `last_sample`, `open_start`, `wm`, decimal strings), and `step(&mut self, now: i64, idle_ms: u64) -> Step`.

`step` algorithm (the part the signature does not determine). Let `input = now - idle_ms` (time of last input) and `reset = last_sample is None || now < last_sample || now - last_sample > COVERAGE_MAX_GAP_MS`.
1. If `reset` and `last_sample` is `Some(l)` and a span is open at `s`: close it as `(s, max(s, l))` (emit only if `end > start`) and clear it. If `reset`: no coverage is emitted; do not use `input` for a new span start, use `now` instead (`input = now`, so an idle already running at the first sample counts from `now`).
2. If not `reset`: coverage is `[(last_sample, now)]`.
3. Open span `s` and `input > s + 2000` (input resumed; 2 s tolerance for timer granularity): close it as `(s, input)`, clear it.
4. If no span is open and `idle_ms >= IDLE_SAMPLE_MIN_MS` (or, after a reset, the idle at `now` is at least that long): open at `max(input, wm + 1)`.
5. `watermark` = open start minus 1 if a span is open, else `now - IDLE_SAMPLE_MIN_MS`; never lower than the previous returned value (`wm`); store it in `wm`. Any span start is clamped to at least `wm + 1` so a backward clock jump cannot create an event at or below a watermark already given.
6. `last_sample = Some(now)` except on a backward jump, where it becomes `Some(now)` too (the next step measures from the new clock).

- [ ] **Step 1: Write failing tests** (helper `run(steps: &[(i64, u64)]) -> (Vec<IdleSpan>, Vec<Interval>, i64)` feeding a fresh `Tracker` and concatenating results; times in seconds times 1000):

```rust
#[test] fn idle_span_starts_at_last_input_and_ends_when_input_resumes() {
    // samples every 5 s: idle 0 at t=0; then idle grows; user types again at t=100 s
    // (0,0) (5,5)... (35,35) opens at input = 0; (95,95) still open; (101, 1) closes
    // expect closed == [IdleSpan{start:0, end:100_000}], none emitted while open
}
#[test] fn open_span_holds_the_watermark_below_its_start() {
    // sample (40s, idle 40s) opens span at 0: watermark == -1; a later sample at 60 s idle 60 s still -1
    // after the span closes at 101 s the watermark is >= 101_000 - 30_000
}
#[test] fn below_threshold_idle_emits_nothing() { /* idle never above 29_999: no spans */ }
#[test] fn coverage_extends_across_normal_samples_and_not_across_a_gap() {
    // samples at 0,5,10 s then 5000 s (sleep) then 5005 s: coverage == [(0,10_000),(5_000_000,5_005_000)]
    // the first sample after the gap emits no coverage
}
#[test] fn resume_after_sleep_does_not_emit_a_span_reaching_into_the_sleep() {
    // samples (0, 0), (40_000, 40_000) open a span at 0; next sample (5_000_000, 5_000_000)
    // closed == [IdleSpan{start:0, end:40_000}] (rule 1: closed at the last sample, 40 s, before the gap) and
    // no span starts before 5_000_000
}
#[test] fn backward_clock_jump_resets_coverage_and_never_goes_below_the_watermark() {
    // samples at 100 s, 105 s (watermark ~75 s), then 50 s: no coverage for (105,50), no span with start <= previous watermark,
    // returned watermark never decreases
}
#[test] fn forward_clock_jump_over_the_gap_limit_is_a_gap() { /* sample at 0 then 61_000 ms: coverage empty */ }
#[test] fn state_roundtrip_continues_an_open_span_after_restart() {
    // step to open a span, save to a map-backed SourceState, load a new Tracker, step 5 s later with still-idle: no event, same watermark rule
    // restart after > 60 s gap: the span is closed at the saved last_sample
}
```

(Map-backed `SourceState`: a test-only `struct MapState(BTreeMap<String,String>)` implementing plan 03's trait.)

- [ ] **Step 2:** `cargo test -p tally-daemon presence`. Expected: FAIL.
- [ ] **Step 3:** Implement `Tracker` per the algorithm.
- [ ] **Step 4:** `cargo test -p tally-daemon`. Expected: PASS, no warnings.
- [ ] **Step 5:** Commit: `feat(daemon): presence tracker with coverage`.

---

### Task 3: Coverage guard in `finalize`

**Files:** Modify `crates/tally-daemon/src/finalize.rs`, `crates/tally-daemon/src/store.rs` (add `watermark(&self, source: &str) -> Result<Option<i64>, StoreError>`), `crates/tally-daemon/src/presence.rs` (add `pub const PRESENCE_SOURCE: &str = "presence";`).

**Interfaces:**
- `FinalizeSummary` gains `pub skipped: usize` (newly skipped by this call). `provisional` becomes `claims.len() - ledger.len() - all_skipped_len` (saturating).
- Rules, for the ledger, the "live capture only" rule and finalization:
  1. The guard is active iff `store.presence_required()`. Off: behaviour is exactly today's (no skips).
  2. Active and the store has no watermark named `PRESENCE_SOURCE`: the horizon is treated as `None`, so nothing is final. (A user whose presence source never ran is blocked and told by `tally run`, not silently skipped.)
  3. A claim is decided once, when it is final (`ts + INTERRUPT_FOLLOW_MS <= horizon`). Its interval is `[start, ts]` with `start = prev.ts` (previous claim of the whole list, skipped or not, which is the previous human action) if there is a previous claim and `ts - prev.ts <= MAX_INTERVAL_MS`, else `ts - FIRST_ACTION_MS`. It is credited iff one stored coverage interval `(s, e)` has `s <= start && ts <= e`. Otherwise it is skipped with reason `no_presence_coverage`.
  4. Why this is safe: the presence runner step writes coverage before it advances the presence watermark, and `finalize` reads the horizon before the coverage, so coverage read is at least what the horizon implies; every final claim has all its coverage in the table when it is decided. Decisions are never revisited, so later coverage cannot resurrect a skipped claim and coverage never makes a ledgered claim diverge.
  5. Ledger and skips are written in the one `append_ledger` transaction. The ledger remains a hash chain over credited claims only. The divergence check compares the stored ledger with the re-scored claims after removing every claim whose `action_id` is in `skipped_claims`; skipped claims were never in the ledger, so removing them is exact. Claims that are neither ledgered nor skipped form a suffix of the claim list (decisions are made in time order).
  6. Live capture: coverage starts at the first healthy presence sample, so time before the daemon first ran, and any time it was off or the machine slept, is outside coverage. A claim whose interval touches such time is skipped in full, not trimmed (conservative; trimming would need a scorer change). Cost: the first prompt within `FIRST_ACTION_MS` of a start, and the first action after a sleep shorter than an hour, earn nothing. That is the accepted price of never over-crediting.

- [ ] **Step 1: Write failing tests** in `finalize.rs` (helpers from `test_support`; helper `guarded()` returns a store with `set_presence_required(true)`; the presence watermark is set with `set_watermark("presence", ...)`):

```rust
#[test] fn uncovered_claim_is_skipped_with_a_reason_and_covered_ones_are_ledgered() {
    // events: prompt a@1_000_000 (rel 600), edit b@1_100_000, edit c@1_900_000
    // coverage [(800_000,1_200_000),(1_500_000,3_000_000)], presence watermark 3_000_000
    // finalize: appended 2, skipped 1; ledger action ids [a, b]; skipped() == [(c, 1_900_000, "no_presence_coverage")]
    // verify_chain(ledger) ok
}
#[test] fn a_skip_is_permanent_and_later_claims_still_chain() {
    // after the case above: extend_coverage(&[(1_200_000, 1_500_000)]); finalize again: skipped 0, ledger unchanged
    // append edit d@2_000_000 (interval starts at c.ts 1_900_000, covered), advance watermark: d is ledgered, no Divergence
}
#[test] fn coverage_boundary_is_inclusive_and_one_ms_short_is_skipped() {
    // edit x@1_000_000 first action: interval [880_000, 1_000_000]; coverage (880_000, 1_000_000) credits; (880_001, 1_000_000) skips; (880_000, 999_999) skips
}
#[test] fn guard_off_changes_nothing() { /* same events as test 1 without set_presence_required: c is ledgered, skipped() empty */ }
#[test] fn required_without_a_presence_watermark_finalizes_nothing() {
    // required, other source watermark 10_000_000, no presence watermark: appended 0, skipped 0, ledger empty
}
#[test] fn sleep_gap_end_to_end_with_the_tracker() {
    // Tracker samples every 5 s from 0 to 600 s, then a 20 min sleep, then samples again from 1800 s to 2400 s;
    // store.extend_coverage from each Step; edits at 300 s (covered, first-action interval needs coverage from 180 s: ok),
    // 500 s (covered), 1900 s (interval [500 s, 1900 s] spans the sleep: skipped), 2000 s (covered);
    // watermark from Step.watermark; assert ledger ids and skipped ids exactly
}
#[test] fn status_reports_skipped_claims() { /* tally_daemon::status::status(&s).skipped_claims == 1 after the first test's setup */ }
```

- [ ] **Step 2:** `cargo test -p tally-daemon finalize status`. Expected: FAIL.
- [ ] **Step 3:** Implement in `prepare` (read order: horizon, then events, then coverage, skipped, ledger). Pass the new skips to `append_ledger`. Update the doc comment of `finalize` with the rules above.
- [ ] **Step 4:** `cargo test -p tally-daemon`. Expected: PASS, no warnings.
- [ ] **Step 5:** Commit: `feat(daemon): only ledger claims inside presence coverage`.

---

### Task 4: Presence source, Windows platform, runner wiring

**Files:** Modify `presence.rs`, `source.rs`, `run.rs` (plan 03), `Cargo.toml`, `lib.rs`; create `presence_win.rs`.

**Interfaces:**
- `source.rs`: add to `trait Source` the default method `fn take_coverage(&mut self) -> Vec<Interval> { Vec::new() }` (drains coverage produced by the last `poll`). The runner calls it right after `poll`, then `store.extend_coverage(&spans)`, and only then `set_watermark`. This ordering is what Task 3 rule 4 relies on; add a runner test: coverage is stored before the watermark moves.
- `presence.rs`:
  - `pub enum PlatformError { Unsupported, Failed(String) }` (`thiserror`)
  - `pub trait Platform { fn idle_ms(&mut self) -> Result<u64, PlatformError>; }`
  - `pub struct PresenceSource<P: Platform>` with `new(p: P) -> Self`, implementing `Source`: `id()` is `PRESENCE_SOURCE`; `poll` loads the `Tracker` from state, calls `idle_ms`, `step(now_ms, idle)`, saves the tracker, turns each closed span into `Event { v:1, id:"presence:idle:<start>", ts:start, source:"presence", project:None, session:None, kind: Kind::Idle{end_ts} }`, stores `Step.coverage` for `take_coverage`, returns `Poll { events, watermark: Some(step.watermark) }`. `Failed` from the platform: return `Poll { events: vec![], watermark: None }` and do not step (the next good sample sees the gap). `Unsupported`: `Err(SourceError)` (type from plan 03) with text `presence is not supported on this platform`.
  - `pub fn default_platform() -> impl Platform`: `WindowsPlatform` on Windows, `UnsupportedPlatform` (always `Unsupported`) elsewhere.
- `presence_win.rs` (`#[cfg(windows)]`): `WindowsPlatform::idle_ms`: `GetLastInputInfo` into `LASTINPUTINFO { cbSize: size_of, dwTime: 0 }`; idle = `(GetTickCount64() as u32).wrapping_sub(dwTime) as u64` (the 32-bit tick wraps every 49 days). Cargo: `[target.'cfg(windows)'.dependencies] windows-sys = { version = "0.59", features = ["Win32_System_SystemInformation", "Win32_UI_Input_KeyboardAndMouse"] }` (check the feature containing `GetLastInputInfo` and `GetTickCount64` with `cargo doc` if the build says an item is missing). Note in a doc comment: `GetLastInputInfo` sees only the current session, so the daemon must run in the user's session, not as a service.
- Config (plan 03 loader): table `[presence]` with `enabled` (default `true` on Windows, `false` elsewhere) and `require` (default equal to `enabled`). `tally run` calls `store.set_presence_required(cfg.require)` at start. If `require` is true and the platform is unsupported, `tally run` exits with an error naming the key.

- [ ] **Step 1: Write failing tests** (`FakePlatform { samples: VecDeque<Result<u64, PlatformError>> }`):

```rust
#[test] fn source_emits_a_closed_idle_event_with_a_stable_id_and_coverage() {
    // samples (now, idle_ms): (0, 0), (40_000, 40_000), (55_000, 55_000), (101_000, 100) (resumed; input = 100_900):
    // events == [Idle{ts:0, end_ts:100_900}] (Tracker rule 3 closes at input = now - idle_ms)
    // id == "presence:idle:0", source == "presence", project None; take_coverage() drains: second call is empty
}
#[test] fn open_span_returns_a_watermark_below_its_start() { /* watermark Some(-1) while idle span from 0 is open */ }
#[test] fn failed_sample_gives_no_watermark_and_no_coverage() { /* Failed: Poll{events:[],watermark:None}, take_coverage empty */ }
#[test] fn unsupported_platform_is_an_error_not_silence() { /* UnsupportedPlatform: poll returns Err with the message */ }
#[cfg(windows)] #[test] fn windows_reports_a_small_idle_value() { /* WindowsPlatform.idle_ms() is Ok(_) */ }
```

- [ ] **Step 2:** `cargo test -p tally-daemon presence`. Expected: FAIL.
- [ ] **Step 3:** Implement. Poll cadence is plan 03's (default 15 s); do not add a second timer.
- [ ] **Step 4:** `cargo test -p tally-daemon` and `cargo clippy -p tally-daemon --all-targets`. Expected: PASS, no warnings. On Windows also run `tally run` for one minute by hand and check `tally status` shows `horizon` moving and `sqlite3 tally.db "select * from coverage"` shows one growing interval.
- [ ] **Step 5:** Commit: `feat(daemon): windows presence source`.

---

### Task 5: Git source (commits, user vs agent)

**Files:** Create `crates/tally-daemon/src/git.rs`; modify `lib.rs`, `store.rs` (add `has_agent_commit_marker`; `projects()` and `repos_to_poll()` are defined once, in Task 6), `run.rs` (register the source), `Cargo.toml` (no new dependency).

**Input from plan 03 (README ruling 1, no change requested):** the Claude adapter emits, for every agent Bash call whose command starts with `git commit`, `Event { id: "claude:<session>:<uuid>:commit", ts: <tool_use time>, source: "claude-code", project: Some(..), kind: Commit { sha: "claude:<tool_use id>", by: Agent, lines_changed: 0 } }`. This plan matches it only by `by = agent`, project and time window; the `sha` value is never read. Ordering is guaranteed by the `claude-code` watermark, not by a lag (rule 2 below).

**Interfaces:**
- Consumes: `Source`, `SourceState`, `SourceError` (plan 03); `repo_identity` from Task 6 is not required here (the runner passes one repository per common dir, see Task 6).
- Produces (`git.rs`):
  - `pub struct RepoRef { pub local_id: String, pub root: PathBuf }`
  - `pub trait AgentCommitLookup { fn is_agent_commit(&self, project: &str, commit_ts: i64) -> bool; fn claude_watermark(&self) -> Option<i64>; }`; implemented by `StoreLookup { store: Store, claude_enabled: bool }` (its own connection to the same database file). `is_agent_commit`: a stored `commit` event with `by = agent` for that project and `ts` in `[commit_ts - AGENT_COMMIT_WINDOW_MS, commit_ts + AGENT_COMMIT_AFTER_MS]` (SQL with `json_extract` on `events.json`; `sha` is ignored). `claude_watermark`: `None` if `!claude_enabled`, else `store.watermark("claude-code")` (the `watermarks` row for source id `claude-code`, Task 3's accessor; `None` when there is no row). Reading the store is the runner-provided lookup, not the source touching the store itself (C1 / ruling 5 are about writes).
  - `pub const AGENT_COMMIT_WINDOW_MS: i64 = 120_000; pub const AGENT_COMMIT_AFTER_MS: i64 = 5_000;`
  - `pub const GIT_LAG_MS: i64 = 60_000;` (fallback only, rule 2)
  - `pub struct RawCommit { pub sha: String, pub ts_ms: i64, pub lines: u32 }`; `pub fn parse_log(out: &str) -> Vec<RawCommit>` for the output of `git log --branches --no-merges --since=<secs> --format=@%H %ct %ce --numstat` filtered by committer email (the email is passed in: `parse_log(out, email)`); lines = sum of added and deleted over numstat rows, `-` (binary) counts 0.
  - `pub struct GitSource { .. }` with `new(repos: Vec<RepoRef>, lookup: Box<dyn AgentCommitLookup>) -> GitSource`, `impl Source`, `id() == "git"`.
- Behaviour of `poll` per repo (state keys prefixed by `local_id`):
  0. Once per poll compute `cutoff`: if `lookup.claude_watermark()` is `Some(cw)`, `cutoff = min(now_ms, cw - AGENT_COMMIT_AFTER_MS)` (a commit at T is emitted only once the `claude-code` watermark is at least T + 5 s, so by C1 every agent marker that could match it is already stored); if `None` (claude-code disabled or no watermark row), `cutoff = now_ms - GIT_LAG_MS`.
  1. First sight of a repo (no `wm` key): set `wm = cutoff`, emit nothing (live capture only).
  2. Else run `git -C <root> log ...` with `--since=<wm/1000 - 1>`; keep commits with `wm < ts_ms <= cutoff` whose committer email equals `git config user.email` (case-insensitive; no email configured: skip the repo). Commits with `ts <= wm` (backdated, imported) are dropped by design. A commit above `cutoff` waits for a later poll.
  3. Event: `id "git:<sha>:commit"`, `ts = ts_ms`, `source "git"`, `project Some(local_id)`, `Commit { sha, by, lines_changed }` with `by = Agent` iff `lookup.is_agent_commit(local_id, ts_ms)`, else `User`. Merge commits are excluded (`--no-merges`) and other people's commits are excluded by the committer filter, so a pull never earns time. Remote-tracking branches are excluded (`--branches`, not `--all`).
  4. A repo's new `wm` is `max(old wm, cutoff)` (never decreases); the source watermark is the minimum over repos; saved to state. A repo whose `git` call fails keeps its old `wm`, which holds the source watermark back (retry next poll).
  5. `git` not on `PATH` (`io::ErrorKind::NotFound`): `Err(SourceError)` with text `git not found on PATH`, no events, no watermark.

- [ ] **Step 1: Write failing tests.**

```rust
#[test] fn parse_log_sums_numstat_counts_binary_as_zero_and_filters_the_committer() {
    let out = "@aaa 1700000010 me@x.org\n3\t1\tsrc/a.rs\n-\t-\timg.png\n\n@bbb 1700000020 other@x.org\n9\t9\tz\n";
    assert_eq!(parse_log(out, "ME@x.org"), vec![RawCommit { sha: "aaa".into(), ts_ms: 1_700_000_010_000, lines: 4 }]);
}
// real-git tests use a tempdir repo; each starts with `if !git_available() { return; }`
// helper commit(dir, msg, unix_secs) runs `git commit --allow-empty`-style commits with GIT_COMMITTER_DATE/GIT_AUTHOR_DATE set
// FakeLookup { agent: Vec<(String, i64)>, claude_wm: Option<i64> } (claude_wm None = claude-code disabled)
#[test] fn first_poll_emits_nothing_then_a_new_commit_is_emitted_once_it_is_older_than_the_lag() { /* claude_wm None */ }
#[test] fn a_commit_inside_the_lag_window_waits_and_the_watermark_is_now_minus_lag() { /* claude_wm None */ }
#[test] fn commit_is_held_until_the_claude_watermark_passes_it_by_five_seconds() {
    // claude_wm Some: commit at T; poll with claude_wm = T + 4_999: no event, source watermark <= T + 4_999 - 5_000;
    // poll with claude_wm = T + 5_000: one event; now_ms - T is far below GIT_LAG_MS (the lag is not used)
}
#[test] fn late_marker_within_the_window_still_makes_the_commit_by_agent() {
    // commit at T, claude_wm = T - 1_000 (adapter behind): no event; then marker (T + 3_000) added and claude_wm = T + 5_000: event by == Agent
}
#[test] fn agent_marker_makes_the_commit_by_agent_and_no_marker_means_user() {
    // FakeLookup agent marker for (project, ts in window): by == Agent; otherwise User. StoreLookup test: marker with
    // sha "claude:toolu_1" matches, any sha value matches, a by=user commit event does not
}
#[test] fn commit_in_a_linked_worktree_is_seen_from_the_main_checkout_exactly_once() {
    // git worktree add ../wt -b feature; commit there; poll the main checkout: one event, id "git:<sha>:commit"
}
#[test] fn other_committers_and_merge_commits_are_ignored() { }
#[test] fn missing_git_is_a_clear_error() {
    // GitSource configured with a `program` override "tally-no-such-git" (a pub(crate) field for tests): Err containing "git not found"
}
#[test] fn failed_repo_keeps_its_watermark_so_the_source_does_not_advance() { /* repo path removed after first poll */ }
```

- [ ] **Step 2:** `cargo test -p tally-daemon git`. Expected: FAIL.
- [ ] **Step 3:** Implement. Run `Command::new(&self.program).args([...])` with `current_dir` unset and `-C <root>`; set `GIT_OPTIONAL_LOCKS=0` so polling never takes the index lock; capture stdout only.
- [ ] **Step 4:** `cargo test -p tally-daemon`. Expected: PASS (git tests silently pass on a machine without git; CI images have git).
- [ ] **Step 5:** Commit: `feat(daemon): git commit source`.

---

### Task 6: Project matching across worktrees and clones, UNC fix

**Files:** Modify `crates/tally-daemon/src/project.rs`, `store.rs`, `main.rs` (stderr note only).

**Interfaces:**
- `normalize_path`: a verbatim UNC prefix (`\?\UNC\` or `\?\UNC\`) becomes `//` followed by the rest, so `\?\UNC\srv\share\d\` and `\srv\share\d` both give `//srv/share/d`. Other verbatim prefixes behave as before. (Behaviour change: ids of projects on network shares, registered with the old code, change; there are none in any ledger yet.)
- `pub struct RepoIdentity { pub common_dir: Option<String>, pub remote: Option<String> }` and `pub fn repo_identity(root: &Path) -> RepoIdentity`: runs `git -C <root> rev-parse --path-format=absolute --git-common-dir` (normalized with `normalize_path`) and `git -C <root> config --get remote.origin.url` (normalized by `normalize_remote`). Any failure, including git missing, gives `None` for that field, never an error.
- `pub fn normalize_remote(url: &str) -> String`: lowercases; strips credentials, a `scheme://`, a trailing `.git` and trailing `/`; turns `git@host:owner/repo` into `host/owner/repo`. So `https://github.com/Sectersion/tally.git` and `git@github.com:sectersion/tally` both give `github.com/sectersion/tally`.
- `Store`: `projects` table gets nullable columns `common_dir`, `remote` (added by `ALTER TABLE` when `PRAGMA table_info(projects)` lacks them); `set_project_identity(&mut self, local_id: &str, id: &RepoIdentity)`; `projects(&self) -> Result<Vec<ProjectRow>, StoreError>` (`local_id, root, public_name, common_dir, remote`, in insertion order); `repos_to_poll(&self) -> Result<Vec<(String, String)>, StoreError>`: one `(local_id, root)` per distinct `common_dir` (the earliest registered; every project with no `common_dir` is its own group). The runner builds the `GitSource` repo list from this.
- `register_git_project(store, path, public_name)`: after registering, compute `repo_identity` and store it. If `public_name` is `None` and the project has no name, adopt the public name of another registered project with the same `common_dir`, else the same non-null `remote`. An explicit name always wins. Adoption happens only at registration time and is never applied to claims already ledgered (finalize maps names when chaining; earlier entries keep what they were chained with), so register a worktree before the daemon first records in it.
- `tally project add` prints the local id on stdout as before, and when a name was adopted prints `matched <name> via common dir` or `via remote` on stderr.

- [ ] **Step 1: Write failing tests** in `project.rs` (real-git ones start with `if !git_available() { return; }`):

```rust
#[test] fn verbatim_unc_normalizes_like_plain_unc() {
    assert_eq!(normalize_path(r"\?\UNC\srv\share\d\"), "//srv/share/d");
    assert_eq!(normalize_path(r"\srv\share\d"), "//srv/share/d");
    assert_eq!(normalize_path(r"\?\C:\x"), "C:/x");
}
#[test] fn remotes_normalize_to_host_owner_repo() {
    assert_eq!(normalize_remote("https://github.com/Sectersion/tally.git"), "github.com/sectersion/tally");
    assert_eq!(normalize_remote("git@github.com:sectersion/tally"), "github.com/sectersion/tally");
    assert_eq!(normalize_remote("https://user:tok@github.com/o/r/"), "github.com/o/r");
}
#[test] fn worktree_and_main_checkout_share_a_common_dir() { /* git init; commit; git worktree add; repo_identity equal on common_dir, different project ids */ }
#[test] fn two_clones_of_one_remote_share_the_remote() { /* git init x2, git remote add origin the same url */ }
#[test] fn registering_a_worktree_adopts_the_name_of_its_main_checkout() {
    // register main with Some("o/r"); register worktree with None: project_names() has both mapped to "o/r"
}
#[test] fn an_explicit_name_wins_and_unrelated_repos_stay_unnamed() { }
#[test] fn identity_without_git_is_none_not_an_error() { /* repo_identity on a dir where `git` fails: both fields None */ }
#[test] fn repos_to_poll_returns_one_repo_per_common_dir() { /* main + worktree + unrelated: 2 rows, main's id for the group */ }
```

- [ ] **Step 2:** `cargo test -p tally-daemon project`. Expected: FAIL.
- [ ] **Step 3:** Implement. Reuse one private `fn git(root, args) -> Option<String>` (trimmed stdout on success) for identity and share it with `git.rs` if that is simpler.
- [ ] **Step 4:** `cargo test -p tally-daemon`. Expected: PASS, no warnings.
- [ ] **Step 5:** Commit: `feat(daemon): match worktrees and clones to one public project; verbatim UNC paths`.

---

### Task 7: User test runs via `tally hook test-run`

Decision: capture user test runs with an explicit wrapper, not by watching shells. It is the lowest-effort robust option: no shell integration to maintain per shell, no command text read from history, works in any terminal. Automatic capture (PSReadLine / prompt hooks, a shell history watcher) is **deferred**; it can be added later without changing the event shape. Agent test runs already arrive from plan 03 and are not touched here.

**Files:** Create `crates/tally-daemon/src/testrun.rs`; modify `lib.rs`, `main.rs`, `tests/cli.rs`.

**Interfaces:**
- `pub fn cmd_hash(args: &[String]) -> String`: lowercase hex SHA-256 of the arguments joined with `\u{1f}`. (The command text itself is never stored, C3.)
- `pub fn test_run_event(args: &[String], exit_code: i32, now_ms: i64, project: Option<String>, pid: u32) -> Event`: `id "shell:<now_ms>:<pid>"`, `source "shell"`, `ts = now_ms` (completion time), `Kind::TestRun { cmd_hash, exit_code, initiator: Actor::User }`.
- Clap shape (README ruling 12): `tally hook` is a subcommand group, `Cmd::Hook { #[command(subcommand)] cmd: HookCmd }` with `enum HookCmd { TestRun { #[arg(last = true, required = true)] args: Vec<String> } }`. This plan adds only `TestRun`; plan 08 adds `HookCmd::Cursor { event }` (`hook cursor <event>`) and `HookCmd::Install { .. }` (`hook install cursor`) to the same enum.
- CLI: `tally hook test-run -- <program> [args...]` runs the program with inherited stdin, stdout and stderr, waits, appends the event (project = `project_id` of the git root of the current directory, `None` outside a repository), and exits with the program's exit code (`1` if it was killed by a signal; a program that cannot be started exits `127` with the OS error on stderr and records nothing). If the database cannot be written the command's result is still returned; a warning goes to stderr.
- No watermark for source `shell`: the event is stamped at completion with the current time, and every other source holds its watermark at or below the present, so the event is never at or below a finalized point. State this in the doc comment on `Cmd::Hook`.
- The long help of `tally hook test-run` documents the aliases: PowerShell `function tt { tally hook test-run -- @args }` and bash/zsh `alias tt='tally hook test-run --'`, used as `tt cargo test`.

- [ ] **Step 1: Write failing tests.**

```rust
// testrun.rs
#[test] fn cmd_hash_is_stable_and_argument_sensitive() {
    let a = |s: &[&str]| cmd_hash(&s.iter().map(|x| x.to_string()).collect::<Vec<_>>());
    assert_eq!(a(&["cargo", "test"]), a(&["cargo", "test"]));
    assert_ne!(a(&["cargo", "test"]), a(&["cargo", "test", "-p", "x"]));
    assert_ne!(a(&["ab", "c"]), a(&["a", "bc"]));
    assert_eq!(a(&["cargo", "test"]).len(), 64);
}
#[test] fn event_is_a_user_test_run_stamped_at_completion() { /* fields as specified, is_human_action() is true */ }
// tests/cli.rs
#[test] fn hook_test_run_passes_the_exit_code_through_and_records_one_user_event() {
    // run `tally --home D hook test-run -- <CARGO_BIN_EXE_tally> --help`: exit 0; then `tally status` has "events: 1"
    // run `... hook test-run -- <CARGO_BIN_EXE_tally> no-such-subcommand`: exit code is non-zero and equals the child's; status "events: 2"
}
#[test] fn hook_test_run_of_a_missing_program_exits_127_and_records_nothing() { }
```

- [ ] **Step 2:** `cargo test -p tally-daemon testrun hook`. Expected: FAIL.
- [ ] **Step 3:** Implement. Use `std::process::Command::status()`; do not capture output.
- [ ] **Step 4:** `cargo test -p tally-daemon`. Expected: PASS, no warnings. Add `windows-latest` coverage is already in `daemon.yml`; no workflow change.
- [ ] **Step 5:** Commit: `feat(daemon): tally hook test-run for user test runs`.

---

## Done when

- `cargo test -p tally-daemon` passes on Windows, macOS and Linux CI (the Windows API test runs only on Windows; git tests need `git`, present on CI images).
- On the maintainer's Windows machine: `tally run` for a working session produces idle spans for real breaks, `coverage` grows while it runs, `tally status` shows `skipped_claims`, and a claim spanning a laptop sleep is skipped with `no_presence_coverage` rather than credited.
- A commit in a linked worktree appears once, attributed to the project's public name; an agent `git commit` is `by: agent`, a manual one `by: user`.
- `tally hook test-run -- cargo test` records one user `test_run` and returns cargo's exit code.

## Decisions I made (maintainer to confirm)

1. **Coverage lives in the store, written through a new default `Source::take_coverage` method** (an amendment to plan 03's trait; sources that do not care ignore it). Alternative rejected: a new event kind for "presence healthy" would change the scoring crate and the ruleset.
2. **`finalize` skips, it does not trim, uncovered claims.** A claim whose whole interval is not inside coverage is skipped permanently with reason `no_presence_coverage`. Cost: the first action within 2 minutes of a daemon start, and the first action after any sleep or restart gap shorter than an hour, earn nothing. Trimming would be a scorer change; a later ruleset could do it.
3. **The guard is on by default on Windows (`[presence] require`)**, and when on, nothing is final until the presence source has a watermark. Off elsewhere until plan 12 adds macOS/Linux idle sources; there, hours are over-credited exactly as the scorer already assumes, so do not publish non-Windows ledgers before plan 12.
4. **Skips are decided once, when a claim becomes final**, and recorded in an append-only `skipped_claims` table in the same transaction as the ledger append. The ledger stays a hash chain over credited claims only. The divergence check compares it with re-scored claims minus recorded skips.
5. **Agent commits are recognised by plan 03's agent `commit` event** (`by: agent`, `sha: "claude:<tool_use id>"`, at the `tool_use` time; README ruling 1) by `by = agent`, project and a window of 120 s before to 5 s after the real commit; the sha is not compared. The git source emits a commit at T only once the `claude-code` watermark is at least T + 5 s; it falls back to a 60 s lag only when claude-code is disabled or has no watermark row. Committer email must equal `git config user.email`; merges and remote-tracking branches never earn time.
6. **Git history before the daemon first sees a repository is never read** (live capture only), and a backdated commit (`ts` at or below the source watermark) is dropped.
7. **User test runs: explicit wrapper `tally hook test-run -- <cmd>` only**; automatic shell capture is deferred.
8. **Worktree and clone matching happens at registration only**, adopting an existing public name through the shared git common dir or normalized `origin` URL. It is not retroactive for claims already ledgered.
9. **A failed idle sample is a gap, not a zero.** With plan 03's 15 s poll, up to two missed polls (samples at most 60 s apart) keep coverage; a longer outage breaks it.

## Assumes from earlier plans

From plan 03 (index contracts C1, C5): `crates/tally-daemon/src/source.rs` with `trait Source { fn id(&self) -> &str; fn poll(&mut self, now_ms: i64, state: &mut dyn SourceState) -> Result<Poll, SourceError>; }`, `struct Poll { events: Vec<Event>, watermark: Option<i64> }`, `trait SourceState { fn get; fn set }`, `SourceError`; `crates/tally-daemon/src/run.rs` (the loop: poll, `append_events`, `set_watermark`, persist state, `finalize`; errors from one source are logged and do not stop the others); `source_state` table; WAL mode; `<home>/config.toml` loader with unknown keys an error; a 15 s default poll interval (`[run] interval_secs = 15`); the Claude adapter emitting the agent `commit` event of README ruling 1 under source id `claude-code` and agent `test_run` events.
From code on `main`: `Store::{open, append_events, events, set_watermark, horizon, register_project, project_names, append_ledger(entries, finalized_through, scored_events), ledger, finalized_through}`, `finalize::{finalize, FinalizeSummary, FinalizeError}`, `project::{normalize_path, project_id, find_git_root, register_git_project}`, `status::{status, Status}`, `tally_scoring::{Event, Kind, Actor, score, chain}`, `ruleset::{IDLE_SAMPLE_MIN_MS, MAX_INTERVAL_MS, FIRST_ACTION_MS, INTERRUPT_FOLLOW_MS}`, `test_support::{prompt, edit, ev}`. Task 1 changes `append_ledger` to take a fourth argument; update every caller and test in the same commit. Final signature (README ruling 8): `pub fn append_ledger(&mut self, entries: &[LedgerEntry], finalized_through: i64, scored_events: usize, skips: &[(String, i64, String)]) -> Result<(), StoreError>` (`skips` = `(action_id, ts, reason)`). Plans 05, 07 and 10 call it as `append_ledger(&entries, finalized_through, scored_events, &[])` when they have no skips; plan 05's test sketches (`s.append_ledger(&[..], 5, 0)`, `(.., 10, 0)` and the by-hand appends) are written with three arguments and must add `&[]`.

## Contract for platform sources (plan 12)

Authoritative per README ruling 13; plan 12 implements against exactly this.
- Trait and error (`crates/tally-daemon/src/presence.rs`): `pub trait Platform { fn idle_ms(&mut self) -> Result<u64, PlatformError>; }`; `pub enum PlatformError { Unsupported, Failed(String) }`. `idle_ms` is milliseconds since the last user input in the user's session.
- Files: `presence.rs` (trait, `Tracker`, `PresenceSource<P: Platform>`, `default_platform()`, `PRESENCE_SOURCE = "presence"`), `presence_win.rs` (`WindowsPlatform`). Plan 12 adds its platforms as sibling `#[cfg(..)]` files (for example `presence_mac.rs`, `presence_linux.rs`) and extends `default_platform()`; it does not change the trait, the tracker or the source.
- Unsupported: `UnsupportedPlatform` returns `Err(PlatformError::Unsupported)`, and `PresenceSource::poll` turns that into `Err(SourceError)` "presence is not supported on this platform" (an error, never silence). `Failed` gives `Poll { events: [], watermark: None }` (a gap).
- Finalization: with `[presence] require = true` and no `presence` watermark, nothing finalizes. Config `[presence] enabled` (default `true` on Windows, `false` elsewhere) and `require` (default = `enabled`); plan 12 turns both defaults on for the platforms it supports. `require = true` on an unsupported platform makes `tally run` exit with an error naming the key.
- Sleep-gap rule: samples more than `COVERAGE_MAX_GAP_MS` (60 s) apart, or a backward clock jump, are a gap: no coverage across it, an open idle span closes at the last sample before it, and a claim whose interval touches the gap is skipped with reason `no_presence_coverage`. Platforms only report idle time; they do not detect sleep themselves.

## Not in this plan

macOS and Linux idle sources, a service that runs in the user session, installer (plan 12); automatic shell capture of user test runs; trimming partially covered claims (needs a ruleset change); publishing and ruleset-bump detection (plan 05); comparing skip decisions in uploads (plan 07, add `skipped_claims` to its divergence work); reading git history from before first run.
