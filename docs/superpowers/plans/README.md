# Tally implementation plans: index, shared contracts, and how to execute

Read this first. Every later plan assumes what is written here. Written 2026-11-23 so the work can continue from a fresh session.

## State of the repo

| Plan | Status |
|---|---|
| `2026-11-22-scoring-crate.md` | Done, on `main`. Crate `crates/tally-scoring` (scoring, hash chain, text features, WASM bindings). |
| `2026-11-22-daemon-core-pipeline.md` | Done, on `main`. Crate `crates/tally-daemon` (SQLite store, `finalize`, `ingest`, `status`, `verify`, `project add`). |
| Plans 03 to 12 below | Written, not started. |

Design documents: `docs/superpowers/specs/2026-11-22-daemon-roadmap-design.md` (goals, non-goals, finalization, badge approach C), `docs/spec/scoring.md` (ruleset `2026.11-0`), `PITCH.md` (product and threat model).

## Order and dependencies

```
03 claude adapter + run loop ──► 04 presence + git ──► 05 publish (badge)      <- MVP: README badge, daily
                │
                ├──► 08 more adapters + heartbeat endpoint
                │
06 server core ──► 07 daemon sync ──► 09 server accounts/profiles/badge service ──► 10 submission + judge
                                                                                     
11 local dashboard   (after 04; reads only the local store)
12 packaging + cross-platform + evidence    (last; touches everything)
```

- **MVP = plans 03, 04, 05.** The maintainer's goal: track their own time on this repo with Claude Code, shown as a daily-updating README badge, ledger published beside it.
- Plans 06 and 09 are the TypeScript server and can be done in parallel with 03 to 05 by a different executor, but plan 07 needs both 03 and 06.
- Execute each plan with `superpowers:subagent-driven-development` in its own worktree off current `main`; merge one plan before starting the next that depends on it.

## Plan files

| # | File | Scope |
|---|---|---|
| 03 | `2026-11-23-03-claude-code-adapter-and-run-loop.md` | `Source` trait, poll runner (`tally run`), persisted source state, Claude Code session-log adapter, relevance/novelty with the `text` feature, config file |
| 04 | `2026-11-23-04-presence-and-git.md` | Windows idle-timer source (idle spans), git source (commits, user vs agent), test-run capture, project matching across worktrees |
| 05 | `2026-11-23-05-publish-badge-and-ledger.md` | Daily publish job: badge endpoint JSON and the ledger to a public location; README badge; `tally publish` |
| 06 | `2026-11-23-06-server-core.md` | `server/` TypeScript service, Postgres schema, ledger upload API, WASM chain verification and re-scoring, hash commits |
| 07 | `2026-11-23-07-daemon-sync.md` | Daemon uploader, periodic hash commit, local-to-server project mapping, offline queue |
| 08 | `2026-11-23-08-more-adapters-and-heartbeat.md` | Codex, Aider, Gemini CLI, Copilot, Cursor adapters; WakaTime-compatible heartbeat endpoint; adapter fixtures and health checks |
| 09 | `2026-11-23-09-server-accounts-projects-profiles.md` | Hack Club Auth sign-in, projects and the Unified Projects DB link, GitHub App and server-witnessed pushes, public profiles, badge service, SSE dashboard |
| 10 | `2026-11-23-10-submission-and-judge.md` | Evidence bundle (daemon, scrubbing, opt-in), self-hosted judge service, labels, re-derivable hours, reviewer view |
| 11 | `2026-11-23-11-local-dashboard.md` | Local web dashboard served by the daemon: bugs fixed, time from failure to fix, session replay |
| 12 | `2026-11-23-12-packaging-cross-platform-evidence.md` | macOS and Linux idle sources, installer, harness auto-detection, service registration, resource measurements, demo/pilot/red-team harness |

## Shared contracts (every plan must follow these)

### C1. Event sources (daemon)

Sources are poll-based so they are simple and testable on every OS. In `crates/tally-daemon/src/source.rs` (created by plan 03):

```rust
pub struct Poll { pub events: Vec<tally_scoring::Event>, pub watermark: Option<i64> }
pub trait Source {
    fn id(&self) -> &str;                              // stable, used as the watermark name
    fn poll(&mut self, now_ms: i64, state: &mut dyn SourceState) -> Result<Poll, SourceError>;
}
pub trait SourceState { fn get(&self, key: &str) -> Option<String>; fn set(&mut self, key: &str, value: &str); }
```

- The runner (`tally run`) loops: for each source `poll`, `append_events`, `set_watermark(source.id(), w)` when `watermark` is `Some`, persist state, then `finalize`. Sources never call `finalize`.
- Watermark contract (daemon plan 02, restated): a source returns watermark `w` only when it will never emit another event with `ts <= w`, including a span whose start is `<= w`. A source with an open span (an `agent_busy` or `idle` still running) keeps `w` below that span's start until it emits the closed span.
- Source state is persisted in a new `source_state(source TEXT, key TEXT, value TEXT, PRIMARY KEY(source,key))` table (plan 03 adds it to `store.rs`, with `get_state`/`set_state`). Cursors (file offsets, last-seen ids) live there, never in event data.

### C2. Event ids

Spec §2 says `id` is a ULID. Sources need ids that are the same every time a log is re-read, so the store's dedup makes re-reading idempotent. **Decision (mine, to confirm):** ids are stable derived strings, `<source>:<native-id>:<kind>` (for example `claude:7f3a...:prompt`); the spec line becomes "unique string; adapters use stable derived ids". The plan 03 first task amends `docs/spec/scoring.md` §2 accordingly. No code depends on ULID ordering: `score()` sorts by `(ts, id)`.

### C3. What events may contain

Only hashes and derived numbers (spec §2). Prompt text, file contents, commands, window titles, URLs never reach the store. Adapters read text, derive numbers (`chars`, `relevance_pm`, `novelty_bytes`, `cmd_hash`, `file_hash`), and drop the text in memory.

### C4. Project identity

Local id = SHA-256 of the canonical git-root path (`tally_daemon::project`). A project's public name (for example `sectersion/tally`) is set with `tally project add --name` and applied to claims before they are chained. Worktrees and clones of the same repository each get their own local id; plan 04 adds matching by git common dir / remote URL so one public name covers them (via `register_project` with the same name for each local id).

### C5. Runner configuration

`<home>/config.toml` (plan 03 creates the loader; `home` as in the CLI: `--home`, `TALLY_HOME`, default `dirs::data_local_dir()/tally`). Each source reads its own table, for example `[claude_code]`. Unknown keys are an error. Missing file means defaults.

### C6. Ledger and publishing

The local ledger is one chain across all projects (`LedgerEntry` from `tally-scoring`). Plan 05 decides publishing a per-project subset: because a subset of one chain does not verify on its own, **the published artifact is a per-project chain**. The daemon keeps one chain per published project as a second ledger table (`published_ledger`, same entry format, `prev_hash` chained within the project), built from the same final claims, so each published chain verifies alone with `verify_chain` and the WASM `verifyChainJson`. Plan 05 owns this table. The server (plans 06 and 07) receives the same per-project chains.

### C7. Server API v1 (plans 06, 07, 09, 10)

JSON over HTTPS, versioned under `/v1`. Auth: `Authorization: Bearer <device-token>` for daemons, session cookie for browsers (plan 09).

- `POST /v1/ledger/{project}/entries` body `{ "prev_hash": "...", "entries": [LedgerEntry...] }`. Server verifies with WASM `verifyChainJson(entries, prev_hash)`; returns `200 {head_hash, count}`; `409 {head_hash}` when `prev_hash` is not the stored head (client resyncs); `422 {index, reason}` for a broken chain.
- `GET /v1/ledger/{project}/head` returns `{head_hash, count}`.
- `POST /v1/ledger/{project}/commit` body `{ "hash": "..." , "ts": ms }` records the daemon's committed head (every `COMMIT_INTERVAL_S` = 300 s).
- `GET /v1/projects/{id}/badge.json` shields.io endpoint JSON (`schemaVersion`, `label`, `message`, `color`).
- Entries are append-only server-side too; a ruleset change starts a new chain (`ruleset` field), and the project's hours are the sum of verified seconds across chains.

### C8. Badge JSON

`{ "schemaVersion": 1, "label": "tracked with Tally", "message": "<N.N> h", "color": "blue" }`, verified hours only, all-time, as decided in the roadmap design.

### C9. Where code lives

`crates/tally-scoring`, `crates/tally-daemon` (Rust workspace), `server/` (TypeScript, Bun, Postgres), `dashboard/` (local dashboard frontend, plan 11), `wasm-smoke/` (existing). CI: one workflow per component under `.github/workflows/`.

## Rulings and decisions already made (do not re-open without cause)

- Live capture only: time before the daemon first runs, and time while it is off, is never scored (the scorer treats missing idle data as presence, which would over-credit). Counting history would be a ruleset change with its own plan.
- Ledger is append-only; a ruleset bump starts a new chain. (The daemon does not yet detect a ruleset bump; plan 05 or the first plan that bumps the ruleset adds that check.)
- A claim is final only when `claim.ts + INTERRUPT_FOLLOW_MS <= horizon` (minimum source watermark). `finalize` and ingestion may run concurrently; a count guard in `append_ledger` protects the append.
- Published timestamps are public and permanent (maintainer accepted).
- Verified hours only on the badge; unverified seconds are never credited automatically.
- A fail→pass loop completes once; a gap over `MAX_INTERVAL` starts a new run (both now in spec §3.1 and §4).

## Carried-forward deferred items

From the plan 01 and 02 reviews, to be picked up by the plan noted:

- `RULESET_VERSION` bump does not start a new chain → plan 05 (check at finalize; refuse to append across rulesets).
- `finalize`'s divergence check ignores `kind`, `reasons`, `features` → plan 07 (compare full entries before upload).
- `status` reads are not in one transaction (display only) → plan 11.
- WAL journal mode for concurrent daemon + CLI → plan 03 (the run loop makes this real).
- Spans: a source that registers with a watermark below the finalized point has promised nothing about the past; the count guard handles the race, the first-run behaviour is "events at or before `finalized_through` are rejected" → document in plan 03 source authoring notes.
- Verbatim UNC (`\\?\UNC\...`) project paths normalize differently from `\\server\share` → plan 04.
- `score()` divergence recovery path (what a user does after `Divergence`) → plan 07 (`tally repair` design).

## Non-code workstreams (from PITCH.md; not plans)

Pilot with 30 to 50 teens, red-team report, resource and cost sheets, the demo replay. Plan 12 builds the measurement harnesses; the studies themselves are human work.

## Rules for writing and executing these plans

- Use the format of `2026-11-22-daemon-core-pipeline.md`: header, Global Constraints, Review Focus, File Structure, tasks with Interfaces and TDD steps, "Done when", "Not in this plan".
- A plan states what it **assumes** from earlier plans by naming the exact functions and files (from this index or from code on `main`). When code on `main` differs from this index, the code wins and the plan notes it.
- A task whose design depends on information that does not exist yet (a vendor log format, a third-party API) begins with a **spike task**: capture real samples, write a fixture, then write the parser against it. Never invent a format.
- Keep every plan executable by an engineer who has not seen this conversation: exact paths, signatures, test names and assertions, commands and expected output.

## Cross-plan rulings (from the consistency review, 2026-11-23)

These override anything in an individual plan that disagrees. Each plan was amended to match.

1. **Agent commits.** Plan 03 emits `commit { by: agent, sha: "claude:<tool_use id>", lines_changed: 0 }` at the `tool_use` time, id `claude:<session>:<uuid>:commit`. Plan 04's git source matches an agent commit by `by = agent` plus a time window (the sha value does not matter) and emits a git commit at time T only once the `claude-code` source watermark is at least T + 5 s; the 60 s lag applies only when the claude-code source is disabled.
2. **Ruleset bump.** Plan 05 adds the guard (`finalize` refuses with `RulesetChanged`). Plan 10 owns the main-ledger epoch change that follows a bump: a chain-start marker (epoch) on `ledger`, `finalize` starting a new chain at `GENESIS_HASH`, and `verify_ledger`, the divergence prefix check and `status` working per chain.
3. **Skipped claims** (plan 04, `skipped_claims`) are first-class: plan 07's full-entry comparison and plan 11's removed-minutes and counts subtract/report them; plan 10's re-derivation input carries `excluded_action_ids` (from `skipped_claims` in range) and the server checks they are exactly the claims missing from the stored chain.
4. **Server migrations** are named `NNN_name.sql` with three digits and run in name order; plan 06 uses `001`+, plan 09 `100`-`103`, plan 10 `104_submissions.sql`.
5. **Sources never touch the store** (C1). Extra per-source data goes through default trait methods the runner persists: `take_coverage()` (plan 04), `take_health()` and `take_agent_writes()` (plan 08).
6. **Runner.** Plan 03 creates `crates/tally-daemon/src/run.rs`: `run_loop(store, cfg, home, stop)` with a list of per-tick hooks (publish tick in plan 05, sync tick in plan 07) and `sources(cfg, home, store)` building the source list. `source.rs` holds the trait, `Poll`, `SourceState`, `SourceError`. Plans 04, 05, 07, 08, 12 use these names.
7. **Claude Code agent edits** (Edit, Write, MultiEdit, NotebookEdit `file_path` from plan 03's tool-use parsing) feed plan 08's heartbeat agent-write filter, so agent edits never become `user_edit` claims.
8. **`append_ledger`** signature stacks: plan 04 adds a fourth argument (`skips`); plans 05 and later call it as `append_ledger(&entries, finalized_through, event_count, &[])` unless they have skips. Plans state the signature they expect and say which plan changed it.
9. **Badge rounding:** floor to one decimal, never above verified time: `floor(verified_s / 360) / 10` hours. Same in plans 05, 06, 09 (contract C8).
10. **Server ruleset policy:** accepts new chains only for its current ruleset, and still accepts appends to an existing chain of an older ruleset (append-only, chain-verified, no re-scoring). Plan 10 lists every place a ruleset string is pinned (fixtures, tests) and how to regenerate them.
11. **Server URLs** use the server's `projects.id`, not the public name (which contains `/`). The daemon maps name to id with plan 07's `server_ids()`. Authorization uses plan 06/09 `requireToken(sql, req, projectId)`; a project's owner comes from `projects.owner_id`.
12. **CLI shape:** `tally hook` is a subcommand group: `hook test-run -- <cmd>` (plan 04), `hook cursor <event>` and `hook install cursor` (plan 08). `tally login --browser` belongs to plan 12.
13. **Presence model (plan 04 is authoritative):** names `Platform`, `PlatformError { Unsupported, Failed }`, files `presence.rs` and `presence_win.rs`, coverage in the store, uncovered claims skipped with reason `no_presence_coverage`, `[presence] enabled/require`. Plan 12's macOS and Linux sources implement plan 04's trait and turn on its defaults for those platforms.
14. **Privacy statements** must list everything that can leave the machine: ledger entries and head hashes (sync, plan 07), badge data (plan 05), and scrubbed session text only on explicit `tally submit` (plan 10).
