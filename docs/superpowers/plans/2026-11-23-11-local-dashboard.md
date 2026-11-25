# Local Dashboard Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `tally dashboard` (and the same server inside `tally run`) serves a localhost-only web page that helps the user get better at building with AI: bugs fixed and time from failure to fix, how often prompts drew on what the user saw when running the code, hours per day and project (verified and unverified apart), what Tally removed from their time and why, and a replay of each session as a timeline of prompt, run, error, fix, commit. Everything is derived from the local store; no text exists to show.

**Architecture:** Three layers. (1) A read-only analytics library in `tally-daemon` (`analytics`, `replay`) over a consistent read snapshot of the SQLite store and ledger, reusing `tally_scoring::caps::completes_fail_pass_loop` and the ledger's `reasons` instead of re-deriving scoring. (2) A pure request router (`api`) from `(method, path, query, Host header)` to a JSON response, with the Host allowlist and read-only rules, tested without sockets. (3) A thin `tiny_http` server (`serve`) that binds `127.0.0.1`, plus a Preact + TypeScript single-page frontend in `dashboard/`, built by Bun and embedded in the binary with `include_dir`.

**Tech Stack:** Rust (existing crate deps) plus `tiny_http` and `include_dir`; `serde` derive for response structs. Frontend: Bun (build and test), TypeScript, Preact, hand-written SVG charts, `happy-dom` and `@testing-library/preact` and `axe-core` for tests (dev only).

**Spec:** `docs/superpowers/specs/2026-11-22-daemon-roadmap-design.md` (section 7), `docs/spec/scoring.md` §2 (events), §3 (hours), §4 (caps, fail→pass loop), §6 (ledger), `PITCH.md` ("Why people keep it installed", Privacy). Read `docs/superpowers/plans/README.md` first.

## Assumes from earlier plans

Code on `main` wins where it differs; note the difference in the task that hits it.

- Plan 02 (on `main`): `Store` (`crates/tally-daemon/src/store.rs`) with `conn: pub(crate)`, `events()`, `ledger()`, `horizon()`, `finalized_through()`, `project_names()`; `status::status`; `finalize`; the CLI in `main.rs`; `test_support::{ev, prompt, edit}`. Events table `events(id, ts, json)` with index on `ts`; ledger table `ledger(seq, hash, entry_json)` with append-only triggers.
- Plan 03: the `tally run` loop in a function this plan can add a thread to; WAL journal mode (`PRAGMA journal_mode=WAL` in `Store::open`); the `<home>/config.toml` loader. If WAL is not yet on, Task 1 turns it on in `Store::open` (idempotent).
- Plan 04: `test_run` and `commit` events exist in real stores. Until then the dashboard works on fixtures and on NDJSON ingested by hand.
- `tally_scoring` public API: `Event`, `Kind`, `Actor`, `LedgerEntry`, `caps::completes_fail_pass_loop(events: &[Event], idx: usize) -> bool` (events sorted by `(ts, id)`, deduplicated), `ruleset::{MAX_INTERVAL_MS, FIRST_ACTION_MS, LOOP_WINDOW_MS, THINK_RELEVANCE_PM}`. Reason strings in claims: `first_action`, `read_protected`, `think_protected`, `idle_subtracted`, `agent_busy_subtracted`, `fail_pass_loop`, `interrupt_followed`, `capped`.
- The scoring crate and the ledger format are not modified by this plan.

## Global Constraints

- Read-only: no HTTP handler, analytics function or replay function writes to the database. Dashboard connections are opened `SQLITE_OPEN_READ_ONLY`. The only write this plan adds is the one-time schema migration of Task 1, run by the read-write `Store::open`.
- No text exists in the store (spec §2, README C3), so the dashboard displays only event kinds, timestamps, counts, seconds, hashes shortened for display, and project names. No file names, commands, prompt text.
- Bind `127.0.0.1` only. Never `0.0.0.0`, never `::`. There is no flag to change the bind address. Port default `7878`; `--port 0` picks a free port (used by tests); the chosen URL is printed.
- Host-header allowlist on every request: `127.0.0.1:<port>` and `localhost:<port>` (the bound port), nothing else; otherwise `403`. No CORS headers are ever sent. Only `GET` (and `HEAD`) are routed; anything else is `405`.
- No runtime network access from the page: no CDN, no web fonts, no analytics. Every response carries `Content-Security-Policy: default-src 'self'; connect-src 'self'; img-src 'self' data:; style-src 'self'; script-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'`, `X-Content-Type-Options: nosniff`, and `/api/` responses also `Cache-Control: no-store`.
- Time: all stored times are UTC ms `i64`. The server never guesses a time zone. The browser sends the instants of the user's local midnights (`edges`, Task 6), so day boundaries follow the user's calendar including DST.
- Pagination and window caps: every list endpoint has a hard limit; no endpoint loads the whole `events` table into memory. Windows are capped at 366 days.
- Size and RAM budget: no async runtime (no `tokio`), two to four server threads, built frontend gzipped under 60 KB, the dashboard adds under 5 MB RSS idle (measured in Task 10, recorded, not gated).
- Everything compiles and tests pass on Windows, macOS and Linux. `bun` is needed only to rebuild the frontend; `cargo build` works without it because the built `dashboard/dist` is committed.
- Accessibility baseline (Task 9): semantic landmarks, one `h1`, keyboard-reachable controls with visible focus, a text alternative (data table) for every chart, no information by colour alone, `prefers-reduced-motion` respected, contrast at least 4.5:1, `lang` set.
- Commit messages follow the repo style (`feat(daemon): ...`, `feat(dashboard): ...`).

## Decisions I made (maintainer to confirm)

1. **`tiny_http` (synchronous, a few threads), not `axum`/`tokio`.** The daemon is synchronous (`rusqlite`), the dashboard serves one user, and an async runtime adds binary size and idle memory against the 50 MB RAM target for no benefit.
2. **Preact + TypeScript, built with Bun, hand-written SVG charts, no chart library.** Preact is about 4 KB gzipped; a charting library is 10 to 100 times that. Output is a small `dist/` (one JS, one CSS, one HTML) embedded with `include_dir`. Rejected: plain TypeScript with no framework (the replay and tables are state-heavy; a 4 KB component model is cheaper than hand-rolled DOM updates), React (about 45 KB), Svelte or Solid (extra compiler toolchain for no gain here).
3. **The built `dashboard/dist` is committed**, so `cargo build` and `cargo install` need no JavaScript toolchain. CI rebuilds and fails on any `git diff` in `dist/` (drift check, Bun version pinned). The alternative (a `build.rs` that runs Bun) makes every Rust contributor install Bun.
4. **The browser sends local-midnight instants (`edges=ms1,ms2,...`) instead of a time zone name.** The daemon then needs no tz database (`chrono-tz` is about 500 KB of data) and DST days of 23 or 25 hours are right by construction. Cost: a longer query string (at most 367 numbers).
5. **"Bugs fixed" is exactly the scorer's fail→pass loop**, by calling `tally_scoring::caps::completes_fail_pass_loop`, not a second definition. The count of loops therefore matches the `fail_pass_loop` reason over all re-scored claims (tested). Plan 04 can skip a claim (`skipped_claims`, `no_presence_coverage`); a loop whose completing run is skipped is **counted separately** (`skipped_fixes`), shown as "skipped", and not added to `fixed`. Only **user-initiated** runs can be ledger-verified: `score()` makes claims only for human actions and a `TestRun` is one only when `initiator == User`, so a loop completed by an agent run has no claim and no `fail_pass_loop` reason. Those are reported separately as `agent_fixes` ("fixed by the agent"), never in `fixed`, never as verified time. So `fixed` equals the `fail_pass_loop` reasons in the ledger, and `fixed + skipped_fixes` equals those in all re-scored claims (tested, with an agent fix in the table). Average and median time to fix include both user and agent fixes (not skipped ones), and the page says so. Time from failure to fix runs from the first failing run of the same `cmd_hash` after the previous passing run of it (within `LOOP_WINDOW_MS`) to the fixing run.
6. **Prompt-after-failure metric** (the "drew on what you saw" proxy), defined precisely in Task 3: among prompts within 5 minutes after a failing `test_run` in the same project, the share with `relevance_pm >= 300`, shown next to the same share for all other prompts so the user sees a difference rather than an absolute number. Shown with its limits on the page.
7. **Minutes removed are derived, not stored.** The ledger records `verified_s`, `unverified_s` and `reasons`, not the interval. The ledger no longer lists every human action: plan 04 records claims outside presence coverage in `skipped_claims` (`Store::skipped()`, action id, ts, reason). The previous human action is the previous ledger entry or the previous skipped claim, whichever has the later `ts`; its `ts` gives the interval (the scorer's rule: first action of a run gets `FIRST_ACTION_MS`; a gap over `MAX_INTERVAL_MS` starts a run). Removed seconds = interval minus `verified_s` minus `unverified_s`, within 1 s of rounding per claim. A skipped claim's whole interval is shown as its own "no presence coverage" amount, never as verified, unverified or removed time. This avoids changing the ledger format (a ruleset change).
8. **Two virtual generated columns on `events`** (`kind`, `project`, from the JSON) with indexes `(kind, ts)` and `(project, ts)`, added by an idempotent migration. They make "test runs in a window" and "events of one project in a window" index scans instead of full-table JSON scans. Cost: a one-time index build on first start after upgrade (seconds per million events) and disk (tens of bytes per event).
9. **Replay sessions are defined by the dashboard, not the harness**: consecutive human actions in one project with gaps of at most 30 minutes (`REPLAY_SESSION_GAP_MS`). A display constant, not a ruleset constant.
10. **No real browser in tests.** The end-to-end test boots the real binary against a fixture store and renders the real served page in `happy-dom` against the live API. It does not test layout or colour contrast; those are on the manual checklist in Task 10. Playwright would add a large browser download to CI for little extra signal here.

## Review Focus

1. **A store with millions of events.** Expected: every endpoint is bounded by a window (default 28 days, at most 366) and uses the `(kind, ts)` or `ts` index; the replay returns at most 2000 items with a `next` cursor; hours and bug counts read the ledger by `seq` range found with a binary search on `ts`, never a full scan. Test in Task 1 (`EXPLAIN QUERY PLAN` uses the index) and Task 5 (cursor pages are disjoint and complete). Measure with a 2,000,000-event generated store in Task 10 and record the response times.
2. **Data arriving while the page is open.** Expected: every response carries `as_of` (horizon and `finalized_through`); the page polls every 15 s while visible and pauses when the tab is hidden; a replay already shown is not re-ordered under the user; only final claims are shown and the page says "up to date through HH:MM"; nothing provisional is mixed into totals. Test in Task 6 (`as_of` present) and Task 9 (component test with a changing fake API).
3. **A project with no fail→pass loops.** Expected: bugs fixed is `0`, average time to fix is `null` (not `0`), the page says so and explains what counts, no division by zero anywhere. Test in Task 2 and Task 9.
4. **Time zones and day boundaries.** Expected: a claim at 23:59:59.999 local belongs to that day and one a millisecond later to the next, on a 23-hour and a 25-hour DST day; edges that are not strictly ascending, more than 367, or that leave the window are `400`. A claim's whole interval is attributed to the day of its end (documented limit). Test in Task 4 and Task 6.
5. **A browser tab left open after the daemon restarts.** Expected: the poll fails, the page shows a calm "Tally is not running, retrying" banner (polite live region), keeps the last data visible, retries with backoff up to 30 s, and recovers without a reload. A new daemon on another port (`--port 0`) cannot be reached by the old tab; after 5 minutes of failures the banner adds "if you restarted Tally, reopen the address it printed". Test in Task 9.
6. **DNS rebinding and other pages talking to localhost.** Expected: a request with `Host: evil.example:7878` is `403` with no database access; `Origin` is not needed for reads because there is nothing to forge (Security notes). Test in Task 6.
7. **Privacy.** Expected: a test scans every JSON response and the embedded page for the fixture's secret-looking strings; `cmd_hash` and `file_hash` are never sent, only shortened ids for display (at most 8 hex characters), and no free text exists. Test in Task 6.
8. **`status` consistency.** Expected: counts, ledger sums and `provisional_claims` come from one read transaction; a concurrent finalize cannot make `final_claims` and `verified_s` disagree. Test in Task 1. The function changed is `tally_daemon::status::status` (Task 1); this closes the carried-forward item.
9. **Skipped claims (plan 04).** Expected: a claim in `skipped_claims` is never verified time; it is the previous human action for the next claim's interval (otherwise the next claim's removed time is inflated by the skipped span); it appears as its own "no presence coverage" row in the timeline and the removed-time breakdown; a bug fix completed by a skipped run is "skipped", counted in `skipped_fixes`, not in `fixed`. Test in Tasks 2, 4 and 5 (fixture with one skipped claim).
10. **Agent-run fixes.** Expected: only user-initiated fixing runs have a claim, so `fixed` equals the ledger's `fail_pass_loop` reasons for those alone; a loop completed by an agent run is `by_agent`, counted in `agent_fixes`, shown as "fixed by the agent", never in `fixed` or verified time, never matched against claims or `skipped_claims`; average and median include it and the page says so. Test in Tasks 2, 5 and 9.

## Security notes (localhost server)

- **Bind** `127.0.0.1` only, so other machines cannot connect. Other local users on a shared machine can; the data is the user's own activity metadata (no text), so the dashboard offers no authentication and `tally dashboard --help` says so. A per-run token in the URL is listed under "Not in this plan".
- **DNS rebinding:** a hostile page can make a browser resolve `evil.example` to `127.0.0.1` and then read responses as same-origin. The Host allowlist defeats it, because the browser sends `Host: evil.example:<port>`.
- **CORS:** no `Access-Control-*` headers, so a cross-origin page cannot read responses with `fetch`. Simple cross-origin requests (an image tag, a form post) can still be sent, but every endpoint is a read-only `GET` with no side effects and every other method is `405`.
- **CSRF** is irrelevant: it needs a state-changing request to forge, and there are none. A later plan that adds one (for example "reassign unattributed time") must add a per-run token and an `Origin` check in that plan.
- **No dynamic HTML:** the frontend renders data as Preact text nodes only (never `dangerouslySetInnerHTML`); project names are the only user-controlled strings and are rendered as text.

## File Structure

```
crates/tally-daemon/
  Cargo.toml                       add tiny_http, include_dir, serde (derive)
  src/lib.rs                       pub mod analytics, replay, api, assets, serve
  src/store.rs                     open_read_only, snapshot, migration, ledger_seq_at_or_after
  src/status.rs                    status() in one read transaction
  src/analytics.rs                 bugs fixed, prompt-after-failure, hours per day/project, time removed
  src/replay.rs                    sessions list and timeline items
  src/dash_api.rs                  pure router: Request -> Response, Host allowlist, JSON (replaces the planned api.rs)
  src/snapshot.rs                  `Snapshot` read transaction and its queries (split out of store.rs)
  src/assets.rs                    include_dir of dashboard/dist, content types
  src/serve.rs                     tiny_http loop bound to 127.0.0.1
  src/main.rs                      `tally dashboard`, `--dashboard` on `tally run`
  tests/dashboard.rs               HTTP test against the real binary (Rust side)
dashboard/
  package.json  tsconfig.json  bunfig.toml
  index.html                       shell with lang and landmarks
  src/main.tsx  src/api.ts  src/time.ts
  src/components/*.tsx             Overview, TimeView, Replay, Onboarding, StatusBanner, DataTable, charts
  src/styles.css
  src/*.test.tsx   e2e/dashboard.e2e.test.ts
  fixtures/seed.ndjson             fixture events (no text)
  dist/                            committed build output (embedded)
.github/workflows/dashboard.yml    bun test, build, drift check, e2e
```

---

### Task 1: Read-only store access, migration, and a one-transaction `status`

**Files:**
- Modify: `crates/tally-daemon/src/store.rs`, `crates/tally-daemon/src/status.rs`

**Interfaces:**
- Produces (`store.rs`):
  - `Store::open_read_only(path: &Path) -> Result<Store, StoreError>`: opens with `OpenFlags::SQLITE_OPEN_READ_ONLY | SQLITE_OPEN_NO_MUTEX`, sets `busy_timeout` 5 s, does not run the schema or the migration. A missing file is an error.
  - `Store::snapshot(&mut self) -> Result<Snapshot<'_>, StoreError>`: starts a deferred read transaction (`BEGIN`); the first read pins the snapshot (WAL gives snapshot isolation). `Snapshot` exposes the same read methods as `Store` that this plan needs: `horizon`, `finalized_through`, `project_names`, `ledger_len`, `ledger_range(seq_from, seq_to)`, `ledger_seq_at_or_after(ts)`, `events_in(kinds, from_ts, to_ts, after, limit)`, `skipped_in(from_ts, to_ts_exclusive)`. Dropping it ends the transaction.
  - `Snapshot::skipped_in(&self, from_ts: i64, to_ts_exclusive: i64) -> Result<Vec<(String, i64, String)>, StoreError>`: the rows of plan 04's `skipped_claims` table (the same shape as `Store::skipped()`: action id, ts, reason) with `ts` in the range, in ts order. Also `Snapshot::last_human_ts_before(&self, ts: i64) -> Result<Option<i64>, StoreError>`: the greater of the last ledger entry `ts` and the last skipped `ts`, both strictly before `ts`.
  - `Snapshot::ledger_len(&self) -> Result<u64, StoreError>`.
  - `Snapshot::ledger_range(&self, from_seq: u64, to_seq_exclusive: u64) -> Result<Vec<(u64, LedgerEntry)>, StoreError>`: rows with `seq` in the range, in order.
  - `Snapshot::ledger_seq_at_or_after(&self, ts: i64) -> Result<u64, StoreError>`: the smallest `seq` (1-based) whose entry `ts >= ts`, or `ledger_len + 1` when none. Binary search over `seq` reading `json_extract(entry_json,'$.ts')` by primary key (`O(log n)` queries); correct because the ledger is a prefix of time-ordered claims.
  - `Snapshot::events_in(&self, kinds: &[&str], from_ts: i64, to_ts: i64, after: Option<(i64, String)>, limit: usize) -> Result<Vec<Event>, StoreError>`: events with `ts` in `[from_ts, to_ts)` and `kind IN kinds`, ordered by `(ts, id)`, strictly after the cursor `(ts, id)` when given, at most `limit`. Uses the `kind` column and the `(kind, ts)` index.
  - Migration inside `Store::init`: if `pragma_table_xinfo('events')` lacks `kind`, run `ALTER TABLE events ADD COLUMN kind TEXT GENERATED ALWAYS AS (json_extract(json,'$.kind')) VIRTUAL`; the same for `project` (`'$.project'`); then `CREATE INDEX IF NOT EXISTS events_kind_ts ON events(kind, ts)` and `events_project_ts ON events(project, ts)`. Also `PRAGMA journal_mode=WAL` (skip for in-memory stores) if plan 03 has not already.
- Modifies (`status.rs`): `tally_daemon::status::status` reads inside one `BEGIN` read transaction (horizon, events count, ledger, finalized_through, and plan 04's `skipped_claims` count); this is the carried-forward "status reads in one transaction" item, done in this task. Its signature does not change; it now takes `&Store` and uses a private helper that runs `BEGIN`/`COMMIT` on `store.conn` (a deferred read transaction on a shared connection is fine because `status` only reads).

- [ ] **Step 1: Write the failing tests** in `store.rs`:

```rust
#[test] fn migration_adds_generated_columns_once_and_old_rows_get_values() {
    // create a db with the OLD schema by hand (events table without kind/project) and one event row,
    // then Store::open(path): `SELECT kind, project FROM events` returns ("prompt", Some("p1"));
    // opening a second time does not error.
}
#[test] fn kind_window_query_uses_the_kind_ts_index() {
    // Store::open_in_memory; EXPLAIN QUERY PLAN for the exact SQL events_in uses;
    // the plan text contains "events_kind_ts" and does not contain "SCAN events" (a SEARCH is fine).
}
#[test] fn events_in_filters_pages_and_never_repeats() {
    // 7 events kinds prompt/test_run mixed, ts 10..70; events_in(["test_run"], 0, 100, None, 2) -> first 2;
    // passing the last (ts,id) as `after` returns the next 2; concatenated pages == all test_run events in order;
    // from_ts is inclusive, to_ts exclusive (an event at ts == to_ts is absent).
}
#[test] fn ledger_seq_at_or_after_binary_search() {
    // ledger with entries at ts 10,20,20,30 (seq 1..4): at_or_after(5)=1, (20)=2, (21)=4, (31)=5, empty ledger -> 1
}
#[test] fn read_only_handle_cannot_write_but_sees_new_commits() {
    // a = Store::open(path); r = Store::open_read_only(path);
    // r.conn.execute("INSERT INTO meta ...") is Err; a.append_events(..) then r.snapshot().events_in(..) sees it
}
```

And in `status.rs`:

```rust
#[test] fn status_is_one_snapshot() {
    // open two handles on one file db; handle A: finalize a store with 2 final claims, then start `status(&b)`
    // via a hook is hard; instead assert structurally: status() issues exactly one BEGIN (use a rusqlite
    // `trace` callback on b.conn counting statements starting with "BEGIN") and the result equals the
    // single-handle status of the same data.
}
```

- [ ] **Step 2:** Run `cargo test -p tally-daemon store status`. Expected: FAIL (items not defined).
- [ ] **Step 3:** Implement to the interfaces above. `Snapshot` borrows `&mut Store` only to hold the transaction; use `conn.execute_batch("BEGIN")` and `COMMIT` on drop (ignore errors in `Drop`). SQL for `events_in` (bind parameters, build the `IN` list from a fixed allowlist of kind strings, never from user input): `SELECT json FROM events WHERE kind IN (...) AND ts >= ?1 AND ts < ?2 AND (ts, id) > (?3, ?4) ORDER BY ts, id LIMIT ?5` with `?3 = i64::MIN`, `?4 = ''` when no cursor.
- [ ] **Step 4:** Run `cargo test -p tally-daemon`. Expected: PASS, no warnings. Existing tests (`status_counts_final_and_provisional_claims` and friends) must still pass unchanged.
- [ ] **Step 5:** Commit: `feat(daemon): read-only store access, kind/project indexes, one-transaction status`.

---

### Task 2: Bugs fixed and time from failure to fix

**Files:**
- Create: `crates/tally-daemon/src/analytics.rs`
- Modify: `crates/tally-daemon/Cargo.toml` (add `serde = { version = "1", features = ["derive"] }`), `crates/tally-daemon/src/lib.rs` (`pub mod analytics;`)

**Definitions (put these in the module doc comment, the page text reuses them):**
- A **bug fixed** is one passing `test_run` that `tally_scoring::caps::completes_fail_pass_loop` accepts: a passing run whose `cmd_hash` matches an earlier failing run within `LOOP_WINDOW_MS`, with a human action in between, and no passing run of that `cmd_hash` between. This is the scoring spec's definition (§4), so the dashboard count equals the number of ledger entries with reason `fail_pass_loop`, once loops whose fixing run was skipped (plan 04, no presence coverage) are set aside: those are `skipped_fixes`, shown as "skipped" and never in `fixed`. This equality holds for **user-initiated** fixing runs only (`by_agent == false`): an agent run (plan 03 ids like `claude:<s>:<uuid>:test_run`, `initiator == Agent`) is not a human action, gets no claim and no ledger reason. A loop completed by an agent run is `by_agent == true`, counted in `agent_fixes`, labelled "fixed by the agent", and never in `fixed`, `skipped_fixes` or verified time. The loop itself is the same (`completes_fail_pass_loop` has no initiator filter and feeds on both).
- **Time from failure to fix** for a bug: the fixing run's `ts` minus the `ts` of the first failing run of the same `cmd_hash` after the previous passing run of it, looking back at most `LOOP_WINDOW_MS`. Milliseconds, `i64`.
- Limits to show on the page: a bug is a failing and then passing run of the same command, so it counts test and build runs, not bugs the user never ran a test for; two unrelated failures of one command count as one bug; a loop longer than one hour is not seen (the scorer's window); runs the agent started count as runs (they feed loops) but the claim time goes to the human.

**Interfaces:**
- Produces:
  - `#[derive(Serialize, Debug, PartialEq)] pub struct BugFix { #[serde(skip)] pub id: String, pub fixed_ts: i64, pub failed_ts: i64, pub project: Option<String>, pub ms_to_fix: i64, pub skipped: bool, pub by_agent: bool }` (`id` is the fixing run's event id; for `by_agent == false` it equals the claim's `action_id`, for an agent fix there is no claim; `by_agent` is `initiator == Actor::Agent` of the fixing run, set by `find_bug_fixes`; `skipped` is false from `find_bug_fixes`, set by `bugs`, and only ever true when `by_agent` is false)
  - `#[derive(Serialize, Debug, PartialEq)] pub struct Bugs { pub fixed: u32, pub skipped_fixes: u32, pub agent_fixes: u32, pub avg_ms_to_fix: Option<i64>, pub median_ms_to_fix: Option<i64>, pub fixes: Vec<BugFix> }` (`fixes` newest first, at most 200, includes skipped ones marked `skipped: true` and agent ones marked `by_agent: true`; `fixed` (user, not skipped), `skipped_fixes` (user, skipped) and `agent_fixes` are full counts and disjoint; avg and median use every non-skipped fix, user and agent, and the page says "includes fixes the agent completed" when `agent_fixes > 0`)
  - `pub fn find_bug_fixes(events: &[Event], from_ts: i64, to_ts: i64) -> Vec<BugFix>`: `events` sorted by `(ts, id)`, deduplicated, containing test runs and human actions with `ts` from `from_ts - LOOP_WINDOW_MS` up to `to_ts`; returns fixes whose `fixed_ts` is in `[from_ts, to_ts)`.
  - `pub fn bugs(snap: &Snapshot, from_ts: i64, to_ts: i64) -> Result<Bugs, StoreError>`: loads `events_in(HUMAN_AND_RUN_KINDS, from_ts - LOOP_WINDOW_MS, to_ts, ...)` in pages of 5000 into one `Vec` (kinds: `prompt`, `interrupt`, `tool_decision`, `user_edit`, `test_run`, `commit`; the excluded kinds are never human actions and cannot change a loop), then calls `find_bug_fixes`, then reads `snap.skipped_in(from_ts, to_ts)` and sets `skipped = true` on every fix with `by_agent == false` whose `id` is a skipped `action_id` (agent fixes are never matched against claims or skipped claims).

- [ ] **Step 1: Write the failing tests** in `analytics.rs`. Helpers build events with `ts` in seconds times 1000: `run(id, s, cmd, exit)` (a `TestRun` by `Actor::User`), `agent_run(id, s, cmd, exit)` (the same with `initiator: Actor::Agent`), `p(id, s)` (a prompt).

```rust
#[test] fn one_loop_is_one_bug_with_time_from_first_failure() {
    // run f1 s=0 exit 1; run f2 s=100 exit 1 (same cmd); p s=150; run ok s=300 exit 0
    // -> fixed 1, ms_to_fix 300_000 (first failure of the streak), avg == median == Some(300_000)
}
#[test] fn no_human_action_between_means_no_bug() {
    // fail s=0 then pass s=60 with nothing between -> fixed 0, avg None, median None
}
#[test] fn a_second_pass_does_not_count_again() {
    // fail, prompt, pass, pass (same cmd) -> fixed 1
}
#[test] fn project_without_loops_has_none_not_zero() {
    // only prompts and passing runs -> Bugs { fixed: 0, skipped_fixes: 0, avg_ms_to_fix: None, median_ms_to_fix: None, fixes: [] }
}
#[test] fn a_loop_completed_by_a_skipped_run_is_skipped_not_fixed() {
    // two loops; the second fixing run's action_id is in skipped_claims (fixture store)
    // -> fixed 1, skipped_fixes 1, fixes has 2 rows with the second skipped == true, avg/median from the first only
}
#[test] fn a_loop_completed_by_an_agent_run_is_an_agent_fix_not_fixed() {
    // user fail s=0, prompt s=30, agent_run pass s=60 (same cmd) -> fixed 0, skipped_fixes 0, agent_fixes 1,
    // fixes == [BugFix { by_agent: true, skipped: false, ms_to_fix: 60_000, .. }], avg == median == Some(60_000);
    // even when the agent run's id is put in skipped_claims, skipped stays false (agent fixes are never matched to claims)
}
#[test] fn a_failure_older_than_the_loop_window_is_not_a_bug() {
    // fail at s=0, prompt, pass at s=3601 -> 0
}
#[test] fn count_equals_fail_pass_loop_reasons_in_scored_claims() {
    // build ~40 events (mixed cmds, passes, fails, prompts) with a fixed seed-free table, including one loop whose
    // fixing run is an agent run (id "claude:s1:u1:test_run", initiator Agent, preceded by a user fail and a user prompt);
    // claims = tally_scoring::score(&events); expected = claims.filter(reasons contains "fail_pass_loop").count();
    // the agent run has no claim, so: find_bug_fixes(..).filter(!by_agent).len() == expected (all claims, skipped or not)
    // and find_bug_fixes(..).filter(by_agent).len() == 1; the agent fix's id matches no claim action_id
    // then skip one fixing claim (record it in skipped_claims, drop it from the ledger fixture) and assert
    // bugs().fixed == ledger entries with the reason, bugs().fixed + skipped_fixes == expected,
    // bugs().agent_fixes == 1 and that fix is in `fixes` with by_agent true, skipped false, and not counted in `fixed`
}
#[test] fn window_filters_by_fixed_ts_but_looks_back_for_the_failure() {
    // failure at s=0, fix at s=4000 is outside LOOP_WINDOW; failure s=3000 fix s=4000, window [3500,5000) -> 1 fix,
    // failed_ts 3_000_000; window [4500, 5000) -> 0 fixes.
}
```

- [ ] **Step 2:** Run `cargo test -p tally-daemon analytics`. Expected: FAIL.
- [ ] **Step 3:** Implement. For each index `i` with `Kind::TestRun { exit_code: 0, .. }` and `fixed_ts in [from_ts,to_ts)` and `completes_fail_pass_loop(events, i)`, walk back through `events[..i]` while `ts` is within `LOOP_WINDOW_MS`, tracking the earliest failing run of the same `cmd_hash` seen before reaching a passing run of it; that is `failed_ts`. Median of an even count is the lower middle (integer). `avg` is the integer mean rounded down. `None` when there is no non-skipped fix (user or agent). Set `by_agent` from the fixing run's `initiator == Actor::Agent`; in `bugs`, `fixed` counts `!by_agent && !skipped`, `skipped_fixes` counts `!by_agent && skipped`, `agent_fixes` counts `by_agent`.
- [ ] **Step 4:** Run `cargo test -p tally-daemon`. Expected: PASS, no warnings.
- [ ] **Step 5:** Commit: `feat(daemon): analytics for bugs fixed and time from failure to fix`.

---
### Task 3: Prompts that drew on what the user saw

**Files:**
- Modify: `crates/tally-daemon/src/analytics.rs`

**Definition (module doc comment, reused on the page):**
- `relevance_pm` (spec §5) is the share, in per mille, of a prompt's words found in the project vocabulary plus the last agent output plus the last error output. The store holds only that number.
- A prompt is **after a failure** when it belongs to a project and the most recent `test_run` of that project before it (any initiator) failed (`exit_code != 0`), at most `AFTER_FAILURE_MS = 300_000` earlier. Every other prompt that has a project is **other**. Prompts with no project are ignored.
- A prompt is **informed** when `relevance_pm >= INFORMED_PM = 300`. These two constants are display constants of this plan, not ruleset constants; they are named in one place and returned in the response (`definition`) so the page prints them.
- Reported for each group: prompt count, informed count, `informed_pm` (informed count times 1000 divided by prompt count, integer, `None` when the group is empty), and `median_relevance_pm`. `enough` is true when the after-failure group has at least `MIN_SAMPLE = 5` prompts; the page shows "not enough yet" otherwise.
- **Limits, stated on the page:** relevance cannot tell words from the error output apart from words that are simply the project's own vocabulary, so a high number after a failure is a hint, not proof, that the prompt used the error; that is why the other group is shown beside it. A prompt zeroed for low novelty counts as not informed. The metric needs `test_run` events (plan 04); with none, the page says there are no test runs to compare against. It measures trend over weeks, not single prompts. Agent-started runs count as runs the user could see.

**Interfaces:**
- Produces:
  - `#[derive(Serialize, Debug, PartialEq)] pub struct PromptGroup { pub prompts: u32, pub informed: u32, pub informed_pm: Option<u32>, pub median_relevance_pm: Option<u32> }`
  - `#[derive(Serialize, Debug, PartialEq)] pub struct PromptsAfterFailure { pub after_failure: PromptGroup, pub other: PromptGroup, pub enough: bool, pub informed_pm_threshold: u32, pub window_ms: i64 }`
  - `pub fn prompts_after_failure(events: &[Event], from_ts: i64, to_ts: i64) -> PromptsAfterFailure`: `events` sorted, deduplicated, kinds `prompt` and `test_run`, starting at `from_ts - AFTER_FAILURE_MS` or earlier; only prompts with `ts` in `[from_ts, to_ts)` are counted; "most recent test run" is per project.
  - `pub fn prompts_after_failure_for(snap: &Snapshot, from_ts: i64, to_ts: i64) -> Result<PromptsAfterFailure, StoreError>`: loads `["prompt", "test_run"]` with `events_in` over `[from_ts - AFTER_FAILURE_MS, to_ts)`.

- [ ] **Step 1: Write the failing tests:**

```rust
#[test] fn a_prompt_right_after_a_failing_run_is_in_the_after_failure_group() {
    // fail run s=0 (project p1); prompt s=60 rel 600 -> after_failure {prompts:1, informed:1, informed_pm:Some(1000)}
}
#[test] fn a_passing_run_in_between_resets_the_group() {
    // fail s=0, pass s=10, prompt s=20 -> other group
}
#[test] fn five_minutes_is_the_edge() {
    // prompt at fail+300_000 is after_failure; at +300_001 it is other
}
#[test] fn runs_and_prompts_in_different_projects_do_not_mix() {
    // fail in p1, prompt in p2 -> other
}
#[test] fn threshold_is_300_per_mille_and_zero_relevance_is_not_informed() {
    // rel 299 not informed, 300 informed, 0 not informed; median of [0, 299, 300] = 299
}
#[test] fn empty_groups_are_none_not_zero_and_enough_needs_five() {
    // no events -> informed_pm None for both, enough false; 4 after-failure prompts -> enough false; 5 -> true
}
#[test] fn window_counts_only_prompts_inside_but_uses_earlier_failures() {
    // failure at from_ts - 60_000, prompt at from_ts + 1000 -> counted as after failure
}
```

- [ ] **Step 2:** Run `cargo test -p tally-daemon analytics`. Expected: FAIL.
- [ ] **Step 3:** Implement with one pass over the sorted events keeping `last_run: BTreeMap<project, (ts, failed)>`. Medians are the lower middle element after sorting; all arithmetic in integers.
- [ ] **Step 4:** Run `cargo test -p tally-daemon`. Expected: PASS, no warnings.
- [ ] **Step 5:** Commit: `feat(daemon): analytics for prompts drawn from what the user saw`.

---

### Task 4: Hours per day and project, and what Tally removed

**Files:**
- Modify: `crates/tally-daemon/src/analytics.rs`

**Definitions:**
- Only **final** ledger entries are used (the ledger). Time not yet final (the last couple of minutes, `INTERRUPT_FOLLOW_MS`) is not shown; the response says up to when it is complete (`as_of.finalized_through`, Task 6). No float and no second scorer.
- A claim belongs to the day containing its `ts` (the end of its interval). An interval that crosses local midnight is attributed wholly to the later day. Stated on the page.
- **Verified** and **unverified** seconds are always separate fields and separate columns/series; they are never added together on the page.
- **Removed seconds** of a claim: `interval_s - verified_s - unverified_s`, floored at 0, where `interval_s` is `FIRST_ACTION_MS / 1000` when the claim has no previous human action or `ts - prev_ts > MAX_INTERVAL_MS`, otherwise `(ts - prev_ts) / 1000`; `prev_ts` is the `ts` of the previous human action across all projects: the previous ledger entry or the previous skipped claim (`skipped_claims`), whichever is later. The ledger alone is not every human action once plan 04 skips claims.
- **Skipped claims** (no presence coverage): a skipped claim's interval (computed by the same rule) is `skipped_s`, reported in its own "no presence coverage" row; it is never in `verified_s`, `unverified_s` or `removed_s`, and a skipped claim has no reasons (they are not stored), so it is not in the "why" breakdown. It still counts as the previous action of the next claim. Accurate to 1 s per claim (the scorer floors each part to whole seconds).
- **Why** a claim lost time, from its `reasons`: `idle_subtracted` and `agent_busy_subtracted` explain removed seconds (key is the sorted `+` join of the reasons present among those two; none present gives key `unexplained`, which should hold only rounding); `capped` explains unverified seconds (moved to `unverified_s`, not removed); `first_action` marks a claim whose interval was the fixed 120 s because the previous action was over an hour earlier (time away longer than that is never credited, so it appears nowhere). `read_protected` and `think_protected` mean time was kept that would have been removed.

**Interfaces:**
- Produces:
  - `#[derive(Serialize, Debug, PartialEq, Default, Clone)] pub struct Seconds { pub verified_s: u64, pub unverified_s: u64, pub removed_s: u64, pub claims: u32 }`
  - `#[derive(Serialize, Debug, PartialEq)] pub struct DayProject { pub day: usize, pub project: Option<String>, pub seconds: Seconds }` (`day` indexes into the request's edges)
  - `#[derive(Serialize, Debug, PartialEq)] pub struct RemovedReason { pub reason: String, pub claims: u32, pub seconds: u64 }`
  - `#[derive(Serialize, Debug, PartialEq)] pub struct Hours { pub days: Vec<DayProject>, pub totals: Seconds, pub removed_by: Vec<RemovedReason>, pub capped: RemovedReason, pub no_presence: RemovedReason, pub no_presence_days: Vec<SkippedDay>, pub protected_claims: u32, pub away_runs: u32 }` and `#[derive(Serialize, Debug, PartialEq)] pub struct SkippedDay { pub day: usize, pub claims: u32, pub seconds: u64 }` (`no_presence.reason` is `"no_presence_coverage"`; not part of `totals`)
  - `pub fn day_index(edges: &[i64], ts: i64) -> Option<usize>`: index `i` with `edges[i] <= ts < edges[i+1]`, `None` outside; binary search.
  - `pub fn hours(entries: &[(u64, LedgerEntry)], skipped: &[i64], prev_ts: Option<i64>, edges: &[i64]) -> Hours`: `entries` consecutive ledger rows within the window, `skipped` the `ts` of skipped claims in the window (ascending), `prev_ts` the `ts` of the human action (ledger or skipped) before the window's first one (or `None`). Entries and skipped are merged by `ts` (a tie puts the ledger entry first) to find each claim's previous human action. `away_runs` counts claims with reason `first_action` after the first (the user came back after more than an hour away).
  - `pub fn hours_for(snap: &Snapshot, edges: &[i64]) -> Result<Hours, StoreError>`: `a = ledger_seq_at_or_after(edges[0])`, `b = ledger_seq_at_or_after(*edges.last())`; reads `ledger_range(a, b)` and `skipped_in(edges[0], *edges.last())`, and takes `prev_ts` from `snap.last_human_ts_before(first_ts_in_window)` (the first ledger or skipped `ts` in the window, whichever is earlier). Rows returned are bounded by the window (at most 366 days).

- [ ] **Step 1: Write the failing tests.** Helper `claim(seq, ts_s, verified, unverified, reasons)` builds a `(u64, LedgerEntry)`.

```rust
#[test] fn day_index_respects_edges_exactly() {
    // edges [0, 100, 200]: ts 0 -> Some(0), 99 -> Some(0), 100 -> Some(1), 199 -> Some(1), 200 -> None, -1 -> None
}
#[test] fn a_claim_one_millisecond_apart_lands_on_different_days_on_a_23_hour_day() {
    // edges built with a 23 h first day (82_800_000 ms): claims at 82_799_999 and 82_800_000 -> days 0 and 1
}
#[test] fn removed_seconds_come_from_the_gap_to_the_previous_claim() {
    // prev ts 0, claim ts 600_000 verified 300 unverified 0 reasons ["idle_subtracted"] -> removed_s 300, removed_by [("idle_subtracted",1,300)]
}
#[test] fn first_action_uses_the_fixed_interval_and_counts_away_runs() {
    // claim after a 2 h gap, verified 90 unverified 0 reasons ["first_action","capped"?]: interval 120 -> removed 30
    // away_runs == 1 for it, 0 for the very first claim
}
#[test] fn capped_seconds_are_unverified_not_removed() {
    // verified 90, unverified 210, reasons ["capped"], interval 300 -> removed_s 0, capped.seconds 210
}
#[test] fn verified_and_unverified_are_never_merged() {
    // two claims, one in each field: totals.verified_s and totals.unverified_s stay separate numbers
}
#[test] fn removed_plus_verified_plus_unverified_equals_the_interval_within_one_second_per_claim() {
    // run tally_scoring::score over a 60-event mixed stream with idle spans, agent_busy and outputs,
    // chain it, then for every claim assert |interval_s - (v + u + removed_s)| <= 1 and unexplained seconds <= claims count
}
#[test] fn a_project_with_no_claims_gives_empty_hours() {
    // hours(&[], &[], None, &[0, 100]) -> days empty, totals all zero, removed_by empty, no_presence zero
}
#[test] fn a_skipped_claim_is_its_own_row_and_is_the_previous_action_of_the_next_claim() {
    // ledger claim A ts 0; skipped claim ts 300_000; ledger claim B ts 360_000 verified 55 unverified 0 reasons ["idle_subtracted"]
    // -> B interval is 60 s (from the skipped claim, not 360 s from A), removed_s 5; no_presence { claims 1, seconds 300 };
    // totals.verified_s only A's + 55; the skipped seconds appear in no totals field; removed_by has no no_presence row
}
#[test] fn skipped_claims_before_the_window_still_set_prev_ts() {
    // hours_for over a fixture store with one skipped claim just before edges[0]: the first in-window claim's interval uses it
}
#[test] fn hours_for_reads_only_the_window() {
    // ledger of 6 entries over 3 days; hours_for with edges of day 2 only returns the entries of that day,
    // and removed_s of the first row of the window uses the previous day's last human action (ledger or skipped).
}
```

- [ ] **Step 2:** Run `cargo test -p tally-daemon analytics`. Expected: FAIL.
- [ ] **Step 3:** Implement. Group `DayProject` rows by `(day, project)` in a `BTreeMap`, output sorted by `(day, project)`. Project is the ledger entry's `project` (the public name when one is registered, otherwise the local id; the router maps ids to a short label, Task 6).
- [ ] **Step 4:** Run `cargo test -p tally-daemon`. Expected: PASS, no warnings.
- [ ] **Step 5:** Commit: `feat(daemon): analytics for hours per day, project and removed time`.

---
### Task 5: Session list and replay timeline

**Files:**
- Create: `crates/tally-daemon/src/replay.rs`
- Modify: `crates/tally-daemon/src/store.rs` (add `Snapshot::project_events_in` and `Snapshot::watermarks`), `crates/tally-daemon/src/lib.rs` (`pub mod replay;`)

**What the user will and will not see (also printed on the replay page):**
- Will see: for each moment, its time, one of these kinds: `prompt`, `run` (a test or build run that passed and is not a fix), `error` (a run that failed), `fix` (the passing run that completed a bug fix, Task 2), `agent_fix` (the same loop completed by a run the agent started: "fixed by the agent", no ledger claim, not verified time), `commit`, `edit`, `interrupt`, `approval`, `denial`, `skipped` (a claim with no presence coverage: its own row at the claim's time, labelled "no presence coverage, not counted"; the event it belongs to is also drawn as usual, and a passing run whose claim is skipped is drawn as `run`, not `fix`), plus bars for `agent_working` and `away` (from `agent_busy` and `idle` spans). Numbers attached: a prompt's `relevance_pm` and length class, a run's exit code, an edit's or commit's `lines_changed`, the first 8 characters of a command hash or commit sha (so repeats of the same command can be matched by eye).
- Will not see: what was typed, what the agent said, which command or file, any diff or window title. Tally never stored them; this is not a redaction that could be turned off.
- Limits: spans that started before the window start are not drawn; `agent_output` events are not drawn (they are numerous and carry only a size); a run that the agent started is shown with an `agent` marker.

**Interfaces:**
- Produces (`replay.rs`):
  - `#[derive(Serialize, Debug, PartialEq)] pub struct Session { pub project: Option<String>, pub start_ts: i64, pub end_ts: i64, pub actions: u32, pub verified_s: u64, pub unverified_s: u64 }` (`start_ts` is the first claim's interval start estimated as the first claim's `ts`; `end_ts` the last claim's `ts`).
  - `pub const REPLAY_SESSION_GAP_MS: i64 = 30 * 60 * 1000;`
  - `pub fn sessions(entries: &[(u64, LedgerEntry)]) -> Vec<Session>`: groups ledger rows by project; a row more than `REPLAY_SESSION_GAP_MS` after that project's previous row opens a new session; result newest first.
  - `pub fn sessions_for(snap: &Snapshot, from_ts: i64, to_ts: i64, limit: usize) -> Result<Vec<Session>, StoreError>`: ledger rows in the window by `seq` range (Task 4 technique), at most `limit` sessions (hard cap 500).
  - `#[derive(Serialize, Debug, PartialEq)] pub struct Item { pub ts: i64, pub end_ts: Option<i64>, pub kind: &'static str, pub n: Option<i64>, pub short: Option<String>, pub agent: bool, pub project: Option<String> }`
  - `#[derive(Serialize, Debug, PartialEq)] pub struct Page { pub items: Vec<Item>, pub next: Option<String>, pub truncated: bool }` (`next` is `"<ts>:<id>"` of the last item when more remain, else `None`).
  - `pub const REPLAY_PAGE_MAX: usize = 2000;`
  - `pub fn replay_page(snap: &Snapshot, projects: &[String], from_ts: i64, to_ts: i64, after: Option<(i64, String)>, limit: usize) -> Result<Page, StoreError>`: `limit` is clamped to `1..=REPLAY_PAGE_MAX`; `projects` are local project ids (empty means all projects, via the `(kind, ts)` index); fix detection uses `find_bug_fixes` over events from `from_ts - LOOP_WINDOW_MS` to the page's last item; `snap.skipped_in(from_ts, to_ts)` supplies the `skipped` items (kind `skipped`, `n` and `short` `None`, `project` `None` because `skipped_claims` stores none; filtered out when a project filter is given) and demotes a `fix` whose id is a skipped `action_id` to `run`. A fix with `by_agent == true` is drawn as kind `agent_fix` (labelled "fixed by the agent", `agent: true`), never `fix`, and is never matched against `skipped_claims`. `sessions` use ledger rows only (skipped claims neither extend nor split a session; stated on the page).
  - `Snapshot::project_events_in(projects: &[String], kinds: &[&str], from_ts, to_ts, after, limit)`: same as `events_in` plus `project IN (...)` (bind parameters), using `events_project_ts`.
  - `Snapshot::watermarks(&self) -> Result<Vec<(String, i64)>, StoreError>`.

- [ ] **Step 1: Write the failing tests** in `replay.rs`:

```rust
#[test] fn sessions_split_on_a_gap_over_thirty_minutes_per_project() {
    // p1 claims at 0, 10 min, 40 min (gap 30 min exactly: same session), 71 min (gap 31: new) ; p2 claim at 5 min
    // -> three sessions, newest first, actions and seconds summed per session
}
#[test] fn replay_maps_kinds_and_numbers_without_text() {
    // events: prompt rel 640; run exit 1; prompt; run exit 0 (same cmd; completes a loop); commit lines 12 by user
    // -> kinds ["prompt","error","prompt","fix","commit"], n [640,1,..,0,12]
    // serialize the page to JSON and assert it contains no key "cmd_hash","file_hash","sha","text"
}
#[test] fn fix_marking_matches_scored_claims_with_the_fail_pass_loop_reason() {
    // same event table as the Task 2 count test; set of items with kind "fix" ts == set of ledger claim ts with that reason;
    // the agent-run fix is exactly one item of kind "agent_fix" with agent true and no matching ledger claim
}
#[test] fn a_skipped_claim_is_its_own_row_and_a_skipped_fix_is_not_a_fix() {
    // fixture store with one skipped claim (the fixing run of a loop): the page has one item kind "skipped" at its ts,
    // the run's own item has kind "run" (not "fix"), and no item with kind "fix" exists for it; with a project filter no "skipped" item
}
#[test] fn spans_become_agent_working_and_away_bars() {
    // agent_busy s=10..20 -> kind "agent_working", end_ts; idle -> "away"; agent_output is omitted
}
#[test] fn pages_are_complete_and_disjoint() {
    // 25 events, limit 10 -> pages of 10,10,5; concatenation equals the unpaged result; `next` None on the last
}
#[test] fn project_filter_returns_only_that_projects_events() { /* two projects, one filter */ }
#[test] fn limit_is_clamped_and_truncation_is_reported() {
    // limit 0 -> treated as 1; limit 10_000 -> REPLAY_PAGE_MAX
}
```

- [ ] **Step 2:** Run `cargo test -p tally-daemon replay`. Expected: FAIL.
- [ ] **Step 3:** Implement. Kinds loaded: `prompt`, `interrupt`, `tool_decision`, `user_edit`, `test_run`, `commit`, `agent_busy`, `idle`. `short` is the first 8 characters of `cmd_hash` or `sha`; `agent` is true when `initiator`/`by` is `agent`. Prompt `n` is `relevance_pm`; run `n` is the exit code; edit and commit `n` is `lines_changed`.
- [ ] **Step 4:** Run `cargo test -p tally-daemon`. Expected: PASS, no warnings.
- [ ] **Step 5:** Commit: `feat(daemon): session list and replay timeline from event kinds`.

---

### Task 6: API router, Host allowlist, JSON

**Files:**
- Create: `crates/tally-daemon/src/dash_api.rs`
- Modify: `crates/tally-daemon/src/lib.rs` (`pub mod api;`)

**Interfaces:**
- Produces:
  - `pub struct Request<'a> { pub method: &'a str, pub path: &'a str, pub query: &'a str, pub host: Option<&'a str> }`
  - `pub struct Response { pub status: u16, pub content_type: &'static str, pub body: Vec<u8>, pub no_store: bool }`
  - `pub struct Api { db: PathBuf, port: u16 }` with `Api::new(db: PathBuf, port: u16) -> Api` and `pub fn handle(&self, req: &Request) -> Response`.
  - Order of checks in `handle`, each with a test: (1) `Host` must be exactly `127.0.0.1:<port>` or `localhost:<port>` (case-insensitive for `localhost`); missing or different gives `403` JSON `{"error":"forbidden host"}` and the database is not opened; (2) method must be `GET` or `HEAD` else `405` with `Allow: GET, HEAD`; (3) route.
  - Routes (all return `{"as_of": {"horizon": i64|null, "finalized_through": i64|null}, "data": ...}`; the snapshot that supplies `as_of` also supplies `data`):
    - `GET /api/status`: `{events, final_claims, provisional_claims, verified_s, unverified_s, sources: [{source, watermark}], projects: [{key, label}]}`; `projects` lists local ids from the `projects` table and any project seen in the ledger; `label` is the public name or the first 8 characters of a 64-hex id. `status()` from Task 1 provides the counts.
    - `GET /api/bugs?from=&to=` gives `Bugs` (Task 2).
    - `GET /api/prompts?from=&to=` gives `PromptsAfterFailure` (Task 3).
    - `GET /api/hours?edges=a,b,c` gives `Hours` (Task 4) with `edges` echoed.
    - `GET /api/sessions?from=&to=&limit=` gives `Vec<Session>` (default limit 100).
    - `GET /api/replay?from=&to=&project=&after=&limit=` gives `Page` (Task 5); `project` is a `label`-style key (public name or local id) resolved to local ids through `project_names` (a name may map to several ids).
  - Validation, each a `400` with `{"error": "<reason>"}` and a test: `from`, `to`, `limit` must be integers; `from < to`; `to - from` at most 366 days in ms; `edges` strictly ascending, 2 to 367 entries, `edges[last] - edges[0]` at most 366 days plus 1 hour (DST), no spaces; unknown query keys ignored; `limit` clamped, never an error. Unknown path under `/api/` is `404` JSON. Database missing or unreadable is `503` `{"error":"no database yet"}` (the onboarding page treats this as "nothing recorded yet").
  - Every response (any status) carries the headers the server adds (Task 7); `Api` returns them in `Response` via a `headers()` method so tests can check them: `Content-Security-Policy` (exact string from Global Constraints), `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`, and for `/api/` `Cache-Control: no-store`. No `Access-Control-*` header is ever produced (test asserts the header list has none).
  - Project key resolution: `fn project_key_to_ids(names: &BTreeMap<String,String>, key: &str) -> Vec<String>`.

- [ ] **Step 1: Write the failing tests** in `api.rs` using a temp database built by `Store::open` and the test-support builders (a fixture with 3 final claims, one fail→pass loop, 2 projects, plus one skipped claim recorded through `append_ledger`'s `skips`; `/api/hours` returns it as `no_presence` and `/api/replay` as a `skipped` item):

```rust
#[test] fn foreign_host_is_403_and_never_opens_the_database() {
    // Api::new(path_that_does_not_exist, 7878): Host "evil.example:7878" -> 403 (not 503)
    // Host "127.0.0.1:7879" (wrong port) -> 403; missing Host -> 403; "LOCALHOST:7878" -> not 403
}
#[test] fn only_get_and_head_are_routed() { /* POST, PUT, DELETE, OPTIONS -> 405 with Allow */ }
#[test] fn options_preflight_gets_no_cors_headers() { /* OPTIONS -> 405, headers contain no access-control-* */ }
#[test] fn status_reports_counts_sources_and_projects_with_as_of() { /* as_of.horizon and finalized_through match the store */ }
#[test] fn bugs_hours_prompts_and_sessions_return_the_analytics_types() { /* parse JSON, compare to calling analytics directly */ }
#[test] fn bad_parameters_are_400_with_a_reason() {
    // from=abc; from>=to; window 400 days; edges descending; edges with 368 entries; edges "1, 2" (space);
    // each -> 400 and error text names the parameter
}
#[test] fn missing_database_is_503_not_a_panic_or_a_500() { /* nonexistent file, valid host -> 503 */ }
#[test] fn no_response_contains_hashes_or_text_from_the_fixture() {
    // fixture uses cmd_hash "SECRETCMDHASH0123456789abcdef" and file_hash "SECRETFILEHASH0123456789abcdef";
    // every route's body must not contain "SECRETCMDHASH0123456789" or "SECRETFILEHASH"
    // (the 8-character `short` prefix "SECRETCM" may appear in replay only)
}
#[test] fn a_100k_event_store_answers_every_route_under_one_second() {
    // generated store of 100_000 events over 28 days (ts-ordered, 1 % test runs), ledger of 5_000 entries;
    // each route's wall time < 1 s in debug mode; marked #[ignore] unless TALLY_SLOW_TESTS is set
}
```

- [ ] **Step 2:** Run `cargo test -p tally-daemon api`. Expected: FAIL.
- [ ] **Step 3:** Implement. Open the store with `Store::open_read_only` per request and take one `snapshot()`. Parse the query with a small hand-written splitter (`split('&')`, `split_once('=')`, percent-decoding is not needed because every parameter value is digits, commas, hex or a project key; reject `%` in `edges` and numeric parameters, decode `%XX` only for `project`, `after`). Serialize with `serde_json`. Errors map: `StoreError::Sqlite(CannotOpen)` to 503, any other store error to 500 `{"error":"internal"}` (log the detail to stderr, never to the response).
- [ ] **Step 4:** Run `cargo test -p tally-daemon`. Expected: PASS, no warnings.
- [ ] **Step 5:** Commit: `feat(daemon): dashboard API router with host allowlist`.

---
### Task 7: HTTP server, embedded assets, and the CLI

**Files:**
- Create: `crates/tally-daemon/src/serve.rs`, `crates/tally-daemon/src/assets.rs`, `crates/tally-daemon/tests/dashboard.rs`, `dashboard/dist/index.html` (a placeholder page containing the text `Tally dashboard (placeholder build)` so `include_dir!` compiles before Task 8; Task 8 overwrites `dist/`)
- Modify: `crates/tally-daemon/Cargo.toml` (add `tiny_http = "0.12"`, `include_dir = "0.7"`), `crates/tally-daemon/src/lib.rs` (`pub mod serve; pub mod assets;`), `crates/tally-daemon/src/main.rs` (new `Dashboard` subcommand; `--dashboard` and `--dashboard-port` on `Run` once plan 03 has it)

**Interfaces:**
- Produces (`assets.rs`): `pub fn get(path: &str) -> Option<Asset>` with `pub struct Asset { pub body: &'static [u8], pub content_type: &'static str }`; `"/"` maps to `index.html`; content types for `html`, `js`, `css`, `svg`, `json`, `png`, `ico`, `txt`; `..` or a leading `//` is `None` (path traversal cannot happen with an embedded directory, the test documents it). Static asset responses use `Cache-Control: no-cache` (they change with the binary) and the same security headers as the API.
- Produces (`serve.rs`):
  - `pub struct DashboardServer { pub addr: std::net::SocketAddr, /* handle */ }` with `pub fn start(db: PathBuf, port: u16) -> std::io::Result<DashboardServer>`: binds `127.0.0.1:port` (`port` 0 picks one; `addr.port()` is then the real port used for the Host allowlist), spawns 2 worker threads that each loop on `Server::recv()` and answer with `Api::handle` or `assets::get`, and `pub fn stop(self)` that calls `unblock` and joins.
  - If `port` is in use: `start` returns the `io::Error`; the CLI prints `port <n> is in use; run tally dashboard --port 0 to pick a free one` and exits 1.
  - Request size limit: tiny_http reads only headers for `GET`; any request with a body is answered `405` without reading it. Per-connection read timeout 5 s (a stalled client cannot hold a worker forever).
- CLI: `tally dashboard [--port 7878] [--no-open]`: starts the server, prints `Tally dashboard: http://127.0.0.1:<port>/`, and blocks until Ctrl-C. It opens the default browser unless `--no-open` or `TALLY_NO_OPEN` is set, using the OS opener (`cmd /c start` on Windows, `open`, `xdg-open`; failure to open is ignored, the URL is already printed). The database must exist; if it does not, the page is still served (the API answers `503` and the page shows onboarding). Opening the read-write `Store` once at start (to run the migration of Task 1) happens before the first request. `tally run --dashboard` (and `dashboard = true` plus `dashboard_port = 7878` in `[run]` of `config.toml`) starts the same server in the same process; its threads share nothing with the run loop except the database file.

- [ ] **Step 1: Write the failing tests.** In `serve.rs`/`assets.rs` unit tests:

```rust
#[test] fn root_serves_index_html_and_unknown_paths_404() { /* assets::get("/") is Some(text/html); get("/nope.js") None */ }
#[test] fn server_binds_loopback_only_and_answers_with_security_headers() {
    // start(db, 0): addr.ip().is_loopback(); raw TcpStream GET / with Host 127.0.0.1:<port> -> 200,
    // headers include Content-Security-Policy, X-Content-Type-Options, no Access-Control-*
}
#[test] fn rebinding_host_is_refused_over_the_socket() { /* Host: evil.example:<port> -> 403 */ }
#[test] fn stop_releases_the_port() { /* start, stop, start on the same fixed free port succeeds */ }
#[test] fn a_post_with_a_body_is_405() { /* raw POST with Content-Length: 5 -> 405 */ }
```

In `tests/dashboard.rs` (drives the real binary, `env!("CARGO_BIN_EXE_tally")`):

```rust
#[test] fn tally_dashboard_prints_its_url_and_serves_status() {
    // build a temp home with the 3-claim fixture via `tally ingest` + `tally finalize`;
    // spawn `tally dashboard --port 0 --no-open --home <dir>`, read stdout until the "http://127.0.0.1:<port>/" line,
    // GET /api/status over a TcpStream with the right Host -> 200 and final_claims == 3;
    // kill the child at the end (guard type that kills on Drop so a failing assert cannot leak a process).
}
#[test] fn dashboard_without_a_database_still_serves_the_page_and_503s_the_api() { /* empty home */ }
```

- [ ] **Step 2:** Run `cargo test -p tally-daemon`. Expected: FAIL (modules and subcommand missing).
- [ ] **Step 3:** Implement. Do not add `tokio`. Compute the allowed Host strings from `addr.port()` after binding. Log nothing per request by default (privacy and noise); `--verbose` prints one line per request with method, path and status (no query string).
- [ ] **Step 3b:** Plan 03 already defines `[run] dashboard` and `dashboard_port` (false, 7878) on `RunConfig`, with its own parse test; read them; CLI flags override. Do not add the fields or a second `config.rs` test here.
- [ ] **Step 4:** Run `cargo test -p tally-daemon` and `cargo clippy -p tally-daemon --all-targets -- -D warnings`. Expected: PASS, no warnings.
- [ ] **Step 5:** Measure and record in the commit message: release binary size before and after (`cargo build --release -p tally-daemon`; expectation under 600 KB added) and idle RSS of `tally dashboard` (Windows: `Get-Process tally | Select WorkingSet64`).
- [ ] **Step 6:** Commit: `feat(daemon): localhost dashboard server and tally dashboard command`.

---

### Task 8: Frontend scaffold, typed API client, local-day edges

**Files:**
- Create: `dashboard/package.json`, `dashboard/tsconfig.json`, `dashboard/bunfig.toml`, `dashboard/index.html`, `dashboard/build.ts`, `dashboard/src/main.tsx`, `dashboard/src/api.ts`, `dashboard/src/time.ts`, `dashboard/src/types.ts`, `dashboard/src/styles.css`, `dashboard/src/time.test.ts`, `dashboard/src/api.test.ts`, `dashboard/.gitignore` (`node_modules/`)
- Modify: `dashboard/dist/*` (build output, committed)

**Interfaces:**
- `package.json`: `"packageManager": "bun@<pinned version, the one installed when this is executed>"`, dependencies `preact` (exact pin), devDependencies `typescript`, `happy-dom`, `@testing-library/preact`, `axe-core`, `@types/bun` (exact pins; commit `bun.lock`). Scripts: `build` (`bun run build.ts`), `test` (`bun test src`), `e2e` (`bun test e2e`), `typecheck` (`tsc --noEmit`).
- `build.ts`: `Bun.build({ entrypoints: ["src/main.tsx"], outdir: "dist", minify: true, target: "browser", naming: "[name].js" })`, then copies `src/styles.css` to `dist/styles.css` and writes `dist/index.html` from `index.html` (script and link tags use relative `main.js` and `styles.css`, no inline script or style, so the CSP `script-src 'self'` holds). `tsconfig.json`: `"jsx": "react-jsx"`, `"jsxImportSource": "preact"`, `"strict": true`.
- `src/types.ts`: TypeScript types mirroring every response struct of Tasks 2 to 6 (`Envelope<T> = { as_of: { horizon: number|null; finalized_through: number|null }; data: T }`, `Bugs`, `PromptsAfterFailure`, `Hours`, `Session`, `Page`, `Item`, `StatusData`).
- `src/time.ts`:
  - `dayEdges(now: number, days: number, tz?: string): number[]`: the `days + 1` instants of local midnight ending with the midnight after `now`'s day, computed with `Intl.DateTimeFormat(undefined, { timeZone: tz, ... })` day parts (never by adding 86,400,000, which breaks on DST days). `tz` is only for tests; the page passes nothing.
  - `formatDuration(seconds: number): string` ("1 h 05 min", "42 s"), `formatClock(ts: number): string` (local `HH:MM`), `dayLabel(edge: number): string` (local date, weekday).
- `src/api.ts`: `class Api { constructor(base = "", fetchImpl = fetch) }` with one typed method per route (`status()`, `bugs(from,to)`, `prompts(from,to)`, `hours(edges)`, `sessions(from,to)`, `replay(...)`); non-2xx responses throw `ApiError { status, message }` (the `503` is `ApiError` with `status 503`); and `poll(fn, { intervalMs = 15000, maxBackoffMs = 30000, onResult, onError }): () => void` which pauses while `document.hidden`, doubles the delay after each failure up to the max, resets after a success, and returns a stop function.

- [ ] **Step 1: Write the failing tests:**

```ts
// time.test.ts
test("dayEdges returns a 23 hour day across a spring-forward change", () => {
  // tz "America/New_York", now = 2026-03-08T18:00Z; the edge pair around 2026-03-08 differs by 23 h
});
test("dayEdges returns a 25 hour day across a fall-back change", () => { /* 2026-11-01, 25 h */ });
test("dayEdges are strictly ascending and have days + 1 entries", () => {});
test("edges for Pacific/Kiritimati (UTC+14) and Pacific/Pago_Pago (UTC-11) still start at local midnight", () => {});
test("formatDuration", () => { /* 42 -> "42 s", 3900 -> "1 h 05 min", 0 -> "0 s" */ });
// api.test.ts
test("api maps a 503 to ApiError with status 503", async () => {});
test("poll backs off after failures, resets after success, and does not fire while hidden", async () => {
  // fake timers via bun's `setSystemTime`/manual scheduler injected into poll; document.hidden toggled
});
test("poll's stop function ends all further calls", async () => {});
```

- [ ] **Step 2:** In `dashboard/`, run `bun install` then `bun test src`. Expected: FAIL (modules not defined).
- [ ] **Step 3:** Implement `time.ts`, `api.ts`, `types.ts`, a `main.tsx` that renders `<h1>Tally</h1>` only, `index.html` (`<html lang="en">`, `<meta name="viewport">`, `<div id="app">`), and `styles.css` with CSS custom properties for colour tokens (light and dark via `prefers-color-scheme`), a visible `:focus-visible` outline, and `@media (prefers-reduced-motion: reduce)` disabling transitions.
- [ ] **Step 4:** Run `bun test src`, `bun run typecheck`, `bun run build`. Expected: PASS; `dist/` contains `index.html`, `main.js`, `styles.css`; `gzip -c dist/main.js | wc -c` is under 20,000 bytes at this stage (record it).
- [ ] **Step 5:** `cargo test -p tally-daemon` still passes with the real `dist/` embedded (the placeholder test text is replaced by `<div id="app">` in the unit test of Task 7; update that assertion to check for `id="app"`).
- [ ] **Step 6:** Commit: `feat(dashboard): scaffold, typed API client, local-day edges`.

---
### Task 9: Views, empty state, onboarding, accessibility

**Files:**
- Create: `dashboard/src/components/App.tsx`, `StatusBanner.tsx`, `Onboarding.tsx`, `Overview.tsx`, `TimeView.tsx`, `Replay.tsx`, `DataTable.tsx`, `BarChart.tsx`, `Timeline.tsx`, and a test file per component (`*.test.tsx`), `dashboard/src/fixtures.ts` (typed sample API responses used by tests)
- Modify: `dashboard/src/main.tsx` (render `App`, hash routing `#/`, `#/time`, `#/replay`), `dashboard/src/styles.css`, `dashboard/dist/*` (rebuild)

**Behaviour (one component each, plain props, the `Api` object injected so tests use a fake):**
- `App`: `<header>` with the one `h1` "Tally", `<nav aria-label="Views">` with three links, `<main>`, `<footer>` with the privacy statement ("This page is built only from event kinds, times and numbers. Tally never stores what you typed, what the agent said, commands or file names."). Polls `status` every 15 s through `poll`. Chooses the view:
  - `ApiError 503` or `status.events === 0` and `final_claims === 0`: **Onboarding**.
  - otherwise the routed view.
- `StatusBanner`: when the last poll failed it renders `<div role="status" aria-live="polite">` "Tally is not running, retrying. Showing what was last loaded (as of HH:MM)." After 5 minutes of continuous failure it adds "If you restarted Tally, reopen the address it printed." It disappears on the first success. When data is fresh it shows "Up to date through HH:MM" from `as_of.finalized_through`, and "Newest minutes appear about 2 minutes after they happen" (the finalization wait).
- `Onboarding` (the first-run and empty state): three numbered steps in an `<ol>`: 1. "Start tracking: run `tally run`" (shown as a `<code>` block), 2. "Add a project: run `tally project add <path>`", 3. "Work with your AI tool as usual; this page fills in within a few minutes." Below, a "What Tally records" list and a "What it never records" list. If `status.sources` is empty it says "No source has reported yet"; if sources exist but `events === 0` it lists each source with "last heard from HH:MM". No chart renders with no data; no zeros are drawn as if they were results.
- `Overview` (default view, three cards in a CSS grid, each a `<section aria-labelledby>`):
  - **Bugs fixed**: the number, "Average time from failure to fix: 12 min" (or "No fixed bugs in this period yet. A fix is a failing test run that later passes after you did something in between."), and a sparkline-free list of the last 5 fixes (time, project, duration; a fix with `by_agent` is labelled "fixed by the agent"). When `agent_fixes > 0` the card adds "N more fixed by the agent (not counted as your time)" apart from the number, and the average line says "includes fixes the agent completed". Period selector: `<select>` with 7, 28, 90 days.
  - **Prompts that used what you saw**: two figures side by side ("after a failing run: 62 %", "other prompts: 41 %") each as text plus a horizontal bar with the number printed; below, the definition text from `informed_pm_threshold` and `window_ms`, the limits paragraph from Task 3, and when `enough` is false "Not enough failing runs yet to compare (5 needed, 2 so far)".
  - **Time Tally removed**: "Of the time between your actions, N min was not counted", the `removed_by` rows as a table ("while you were away from the keyboard", "while the agent worked and you were idle", "both"), the `capped` row ("N min exceeded the cap for that kind of action and is unverified"), and the note that a gap over an hour is never counted at all (`away_runs` times); a separate row "no presence coverage, not counted" from `no_presence` ("N min of actions could not be checked against your keyboard presence"), never listed as verified time or inside the removed total. The bugs card shows "N skipped (no presence coverage)" when `skipped_fixes > 0`, not counted in the number.
- `TimeView`: stacked-by-project daily bars from `hours` with verified in the solid fill and unverified hatched (a pattern, not colour alone), a legend, and `DataTable` under the chart (day, project, verified, unverified, removed). A project selector and the 7/28/90 selector. Verified and unverified are different series and never summed in any label.
- `Replay`: left, the session list (`<ul>` of buttons: project, date, start to end, verified minutes); right, the `Timeline`: a vertical `<ol>` of items, each `<li>` with time, an icon with a text label (prompt, run, error, fix, commit, edit, interrupt, approval, denial, skipped as "no presence coverage"), the attached numbers, and bars for `agent_working` and `away`. A "Load more" button follows `next`. Items already shown are never reordered when new data arrives; new items append and a polite live region announces "N new moments". A panel "What you see here and what you do not" repeats the will/will-not text of Task 5.
- `DataTable`: a real `<table>` with `<caption>`, `<th scope>`; every chart has one (it is the chart's text alternative; the SVG gets `role="img"` and `aria-labelledby` pointing at a title and `aria-describedby` at the table).
- Keyboard: all interactive elements are native (`a`, `button`, `select`); the session list and "Load more" are reachable by Tab; focus moves to the `h2` of a view after navigation (`tabindex="-1"` and `.focus()`); no keyboard trap.

- [ ] **Step 1: Write the failing tests** (`bun test src`, `happy-dom` registered in `bunfig.toml` preload, `@testing-library/preact` for rendering and queries by role):

```tsx
test("onboarding shows when the API says 503 and when there are no events", async () => {});
test("onboarding lists the three steps in order and the never-recorded list", () => {});
test("a project with no fail-pass loops says so, shows 0 fixed and no average, and never renders NaN or 0 min", () => {
  // bugs {fixed:0, agent_fixes:0, avg_ms_to_fix:null, ...} -> text matches /No fixed bugs in this period yet/ and not /NaN|Infinity/
});
test("an agent fix is labelled and kept out of the fixed number", () => {
  // bugs {fixed:2, agent_fixes:1, fixes:[..., {by_agent:true}]} -> number "2", text /1 more fixed by the agent/ and /includes fixes the agent completed/,
  // the agent row has the label "fixed by the agent"
});
test("average time to fix is formatted from milliseconds", () => {});
test("prompt comparison shows not-enough-yet below 5 prompts and prints the thresholds from the response", () => {});
test("removed time table shows each reason with minutes and the capped row separately", () => {});
test("no presence coverage is its own row, not in verified or removed totals", () => {});
test("time view keeps verified and unverified as separate series and table columns", () => {});
test("every chart has a data table and an accessible name", () => {});
test("the whole app passes axe with no violations (rules: all except color-contrast, which happy-dom cannot compute)", async () => {
  // run axe-core on the rendered Overview, TimeView, Replay (with fixtures) and Onboarding
});
test("exactly one h1 and landmarks header, nav, main, footer exist", () => {});
test("data arriving while open: a second poll with more replay items appends them, keeps existing order and announces the count", async () => {});
test("daemon restart: poll fails, banner appears with role=status, last data stays visible; next success removes the banner", async () => {});
test("after five minutes of failures the banner adds the reopen-the-address hint", async () => {
  // injected clock
});
test("polling pauses while the tab is hidden and resumes when visible", async () => {});
test("project names are rendered as text, not HTML", () => {
  // a project label "<img src=x onerror=alert(1)>" appears as literal text; no img element is created
});
test("changing the period requests new edges and from/to for the local calendar", async () => {});
```

- [ ] **Step 2:** Run `bun test src`. Expected: FAIL.
- [ ] **Step 3:** Implement the components to the behaviour above. Charts are plain SVG built in the component (`rect` per bar, scaled with integer maths, a baseline and 3 ticks); no animation unless `prefers-reduced-motion: no-preference`, and even then none by default. Colours come from CSS custom properties with at least 4.5:1 contrast against the background in light and dark; the hatched pattern is an SVG `<pattern>`.
- [ ] **Step 4:** Run `bun test src`, `bun run typecheck`, `bun run build`. Expected: PASS; `gzip -c dist/main.js | wc -c` plus the CSS gzipped is under 60,000 bytes (the Global Constraints budget); record the figure.
- [ ] **Step 5:** `cargo test -p tally-daemon` passes with the rebuilt `dist/`.
- [ ] **Step 6:** Commit: `feat(dashboard): overview, time, replay, onboarding, accessibility`.

---

### Task 10: End-to-end test, CI, measurements, manual checklist

**Files:**
- Create: `dashboard/fixtures/seed.ndjson` (about 60 events across 3 days and 2 projects; includes two fail→pass loops in one project and none in the other, and one claim with no presence coverage (presence required, a gap in `coverage`) that the e2e test sees as "no presence coverage" and not in verified time; every `cmd_hash`/`file_hash`/`sha` is a fixed hex string; no text), `dashboard/e2e/dashboard.e2e.test.ts`, `dashboard/e2e/gen-large.ts` (writes a 2,000,000-event NDJSON for the manual measurement), `.github/workflows/dashboard.yml`
- Modify: none besides `dashboard/dist` if the drift check finds a change

**End-to-end test (`bun run e2e`):** in `beforeAll`, make a temp home; run the real binary (`cargo build -p tally-daemon` was done by the workflow; the test uses `../target/debug/tally` or `TALLY_BIN`): `tally ingest fixtures/seed.ndjson --source seed`, `tally project add` is not needed (events carry project ids), `tally finalize`; spawn `tally dashboard --port 0 --no-open`, read the URL from stdout; register `happy-dom` `Window` with `url` set to that URL and `fetch` set to the real `fetch`, load the served `/` HTML, run its `main.js` from the served `/main.js` by `window.eval`, wait for the app.

```ts
test("served page boots against the live API and shows the fixture's numbers", async () => {
  // within 3 s: the "Bugs fixed" card shows "2" and the average formatted from the fixture's known 5 min + 9 min
  // (average 7 min); "Time Tally removed" lists "while you were away from the keyboard"
});
test("a project filter with no loops shows the empty-bugs message", async () => {});
test("replay lists the fixture session and its first moment is a prompt, the fix moments are marked fix", async () => {});
test("the page and every API response contain none of the fixture's secret strings", async () => {
  // fetch every /api route directly and the index.html and main.js: none contains the fixture's cmd_hash/file_hash full values
});
test("the server refuses a foreign Host header and sends no CORS header", async () => {
  // Bun's fetch cannot override Host, so use a raw `net.connect` socket with `Host: evil.example:<port>` -> "403"
});
test("restarting the daemon on the same port brings the open page back without reload", async () => {
  // kill the child, assert the banner role=status appears within 20 s (poll interval shortened by `?poll=200` test hook
  // main.tsx reads the poll interval from a <meta name="tally-poll-ms"> that the test sets before boot; default 15000), start `tally dashboard --port <same>`, assert recovery
});
afterAll(() => { /* kill child, remove temp home */ });
```

**CI (`.github/workflows/dashboard.yml`):** on push and pull request, paths `dashboard/**`, `crates/tally-daemon/**`, `.github/workflows/dashboard.yml`. Job `frontend` on `ubuntu-latest`: `actions/checkout@v4`, `oven-sh/setup-bun@v2` with the pinned version, `bun install --frozen-lockfile`, `bun run typecheck`, `bun test src`, `bun run build`, then the **drift check** `git diff --exit-code -- dashboard/dist` (fails with "dashboard/dist is stale: run bun run build and commit"), then gzip size check script (fails above 60,000 bytes). Job `e2e` (needs `frontend`) on `ubuntu-latest` and `windows-latest`: checkout, `dtolnay/rust-toolchain@stable`, `Swatinem/rust-cache@v2`, `cargo build -p tally-daemon`, setup-bun, `bun install --frozen-lockfile`, `bun run e2e` with `TALLY_BIN` set. The existing `daemon.yml` already runs `cargo test -p tally-daemon` on three systems and now embeds `dist/`, so no change there.

- [ ] **Step 1:** Write `seed.ndjson` and the e2e tests above. Expected numbers are computed by hand from the fixture and written as comments next to the assertions.
- [ ] **Step 2:** Run `cargo build -p tally-daemon` then `cd dashboard && bun run e2e`. Expected: FAIL first on any assertion the fixture does not yet satisfy; fix the fixture or code until PASS. The restart test must pass 5 times in a row (`for i in 1 2 3 4 5`) without flake.
- [ ] **Step 3:** Write `dashboard.yml`. Verify locally what can be verified (`git diff --exit-code -- dashboard/dist` after `bun run build`); the workflow itself is verified by its first run on the pull request.
- [ ] **Step 4: Measurements (recorded in the commit message, not gated).** Generate the large store (`bun run e2e/gen-large.ts > /tmp/large.ndjson`, 2,000,000 events over 90 days; `tally ingest` it and `tally finalize` it). Record: first start after migration (index build seconds), then median of 5 requests each for `/api/status`, `/api/bugs`, `/api/prompts`, `/api/hours` (28-day edges), `/api/replay` (one day), peak RSS of `tally dashboard` during them, and idle RSS. If any 28-day route exceeds 1 s on the developer laptop, stop and report; do not add caches without a decision.
- [ ] **Step 5: Manual checklist (once, on Windows, in a real browser; results written in the pull request description).** Keyboard only: reach every control, Tab order follows reading order, focus is visible. 200 % zoom and a 320 px wide window: no horizontal scroll of the page, no clipped text. Windows High Contrast and OS dark mode: readable, hatched pattern visible. Screen reader (NVDA or Narrator): headings list, table captions, the banner announcement. DevTools Network tab: no request leaves `127.0.0.1`. Contrast: DevTools or axe extension reports no contrast failures on the four views.
- [ ] **Step 6:** Commit: `test(dashboard): end-to-end test, CI workflow, measurements`.

---

## Done when

- `cargo test -p tally-daemon` passes on Windows, macOS and Linux in CI with the real `dashboard/dist` embedded, including the Host allowlist, 405, no-CORS, and no-text tests.
- `bun test src` and `bun run e2e` pass in CI; the drift check proves `dashboard/dist` matches the sources.
- `tally dashboard` on a store with the fixture shows bugs fixed with the right count and average time to fix, the prompt comparison with its limits, hours per day and project with verified and unverified apart, the removed time with reasons, and a replay; on an empty home it shows the onboarding view.
- `tally status` reads in one transaction (test in Task 1).
- Size and memory numbers (binary growth, gzipped frontend under 60 KB, idle and peak RSS, request times on 2,000,000 events) are recorded in the Task 7 and Task 10 commit messages.

## Not in this plan

- Any write action from the page (for example claiming away time with one click, reassigning unattributed time): needs a per-run token and `Origin` check first, and its own plan.
- A per-run access token in the URL for shared machines; authentication of any kind.
- Showing claims that are not yet final (the last couple of minutes); the page shows final data and says through when.
- Harness health checks per adapter (PITCH.md says the dashboard shows one): plan 08 owns the health data; this plan only lists the sources and their last watermark.
- Public profiles, SSE, and any server-side dashboard (plan 09).
- Pre-computed rollups or caches for stores much larger than 2,000,000 events; add only if the Task 10 measurements say so.
- Windows tray icon or autostart of the dashboard (plan 12).
