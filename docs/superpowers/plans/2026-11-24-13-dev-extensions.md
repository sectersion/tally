# Developer-tool extensions (install once, works forever) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Installing one extension per tool (a Claude Code plugin, a VS Code extension) is the whole setup: the daemon is fetched, started and kept running by the extension, with no `tally` CLI steps and no config editing.

**Architecture:** One new binary command, `tally ensure`, does everything an extension needs (consent record, config defaults, start the daemon detached, register the current repo). Extensions are thin shells that call it and contain no tracking logic, so the behaviour is tested once in Rust. The binary itself comes from plan 12's release artifacts and installers; the extensions vendor those installers instead of writing a second downloader.

**Tech Stack:** Rust (`crates/tally-daemon`), Claude Code plugin format (JSON manifests, hooks, slash commands), TypeScript VS Code extension, the plan 12 shell and PowerShell installers.

**Spec:** the maintainer's request of 2026-11-24 ("just do plugin install and it works instead of a bunch of CLI stuff; Hackatime is install-once, works forever"), `PITCH.md` (privacy promises, threat model), `docs/adapters/heartbeat.md` (editor heartbeat endpoint), plan 12 Part B and Tasks 11 and 13 (explainer, service, release artifacts, installers).

## Global Constraints

- **Install once.** After the extension is installed, no step is required from the user: no `tally init`, no `tally project add`, no editing of `config.toml` or `~/.wakatime.cfg`.
- **Never break the host tool.** An extension hook or activation that fails (offline, binary missing, home unwritable) exits 0 and logs to `<home>/ensure.log`; it never blocks a Claude Code session or an editor start, and finishes within 5 seconds.
- **Privacy is unchanged** (`PITCH.md`): extensions send nothing but heartbeat metadata to `127.0.0.1`, never prompt text or file contents, and have no telemetry of their own. The explainer text must match what the code captures (plan 12 Task 7 Step 6 checks this).
- **Consent.** Plan 12 decision 4 says installers do not start tracking. Here the extension install is the consent act; the first run shows the explainer (see Decision 1) and records `<home>/consent.json`. The extensions start nothing before that file exists.
- **One daemon per home** (`run.lock`, plan 12 Task 10, already in `tally run`). `ensure` may be called any number of times, concurrently, from several tools.
- **Windows first**, as for plans 03 to 05. macOS and Linux work only for what plan 12 Tasks 2 to 5 provide (presence).
- Repo files use LF. Extension code does not add dependencies to `crates/tally-daemon`; `ensure` uses `std::process` only.
- Commits end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`; never commit secrets or signing material.

## Review Focus

1. **Offline or broken install.** No network, a corrupt download, or an unwritable home: the hook still exits 0 within 5 s and the session starts. Tested in Task 5 (`offline_install_fails_quietly`).
2. **Two tools start at once.** A Claude Code session and a VS Code window both run `ensure` in the same second: exactly one `tally run` survives. Tested in Task 1 (`concurrent_ensures_leave_one_daemon`).
3. **Version skew.** An old `tally` on PATH that lacks `ensure`: the extension installs its pinned version into `<home>/bin` and uses that, instead of failing on an unknown subcommand. Tested in Task 5 (`old_binary_is_replaced_not_called`).
4. **Paths with spaces or non-ASCII** (a Windows user name like `Jo Smith`, a project in `D:\My Code\proj é`): the hook runner, the spawn and the `--register-cwd` argument all survive them. Tested in Tasks 1 (`register_cwd_registers_an_unnamed_project_once`) and 5 (`calls_ensure_with_the_cwd`).
5. **Stop must stick.** After `tally stop`, a new session or editor start does not bring the daemon back until `tally start`. Tested in Task 2 (`stop_pauses_and_ensure_respects_it`).
6. **Typing floods and non-files.** The VS Code extension sends at most one heartbeat per file per two minutes unless a save happens, and nothing for untitled, remote (`vscode-remote:`), output-channel or settings documents. Tested in Task 6 (`isTrackable`, `Throttle`).

---

## Design

```
Claude Code session start ──► plugin hook ──┐
VS Code window start      ──► extension ────┼──► tally ensure ──► consent.json, config defaults,
                                            │                    detached `tally run`, repo registered
VS Code edits ──► heartbeats ──► 127.0.0.1:<port> (the daemon's existing WakaTime endpoint)
```

- The daemon already reads Claude Code logs without any hook, so the plugin's only jobs are to get the daemon running and to show the user what happens.
- Daemon lifetime is lazy: it is started by the first session or editor window after boot and keeps running until stopped or the machine restarts. Always-on from boot is plan 12's `tally service install`, unchanged and optional. Hours between boot and the first session are not lost: git commits are read from `git log` by watermark, and idle time with no tool open earns nothing anyway.
- Claude Code registers the current repo as an unnamed, local-only project (nothing is published or synced until the user names or links it), because the git source only reads registered repos.

## Decisions to confirm

1. **Install is consent.** The first `ensure` shows the explainer once (as a hook `systemMessage`, or as a VS Code notification) and records `consent.json`. This replaces the `tally init` prompt for extension users; `tally init` stays for CLI installs.
2. **Heartbeat server on by default** (`[heartbeat] enabled = true`). It binds `127.0.0.1` only and needs the token the daemon creates, so the VS Code extension works without a config edit. A busy port disables only that source (existing behaviour).
3. **Lazy start, not an OS service,** as above.
4. **A custom VS Code extension, not the WakaTime plugin.** Asking users to install the WakaTime plugin and edit `~/.wakatime.cfg` is exactly the CLI step this plan removes. The extension becomes the first real client of the heartbeat protocol, which closes the SYNTHETIC / UNVERIFIED status in `crates/tally-daemon/tests/fixtures/heartbeat/NOTES.md` for our own client only; stock WakaTime plugins stay unverified.
5. **Distribution:** plugin through a marketplace file in this repo (`/plugin marketplace add sectersion/tally`, `/plugin install tally@tally`); VS Code extension as a `.vsix` attached to each release, with the VS Code Marketplace and Open VSX listing left to the maintainer (needs a publisher account).
6. **JetBrains, Neovim and other editors are out of scope.** Add one only when someone uses it; each is a thin client of the same endpoint.

## Prerequisites

Tasks 1 to 4 have no dependencies and can start now; until plan 12 produces releases, test with `tally` on PATH (`cargo install --path crates/tally-daemon`) or with `TALLY_RELEASE_BASE=file://<dir>`. The installing branch of Task 5's hook, Task 6's "Install Tally" button and all of Task 7 need plan 12 Task 11 (`release.yml`: artifacts `tally-<version>-<target>.zip|.tar.gz`, `SHA256SUMS`) and Task 13 (`install/install.sh`, `install/install.ps1`, honouring `TALLY_RELEASE_BASE`, `TALLY_VERSION`, `TALLY_INSTALL_DIR`, idempotent). Until then the plugin works for anyone who already has `tally` on PATH. Plan 12 Task 7 must reuse this plan's `explainer.txt` (Task 1 Step 7).

## File structure

```
crates/tally-daemon/src/ensure.rs        ensure(), daemon_running(), consent file, detached spawn
crates/tally-daemon/src/explainer.txt    the one-page explainer (plan 12 Task 7 reuses it)
crates/tally-daemon/src/main.rs          adds `ensure`, `stop`, `start`, `status --brief`
crates/tally-daemon/src/run.rs           run loop exits when `<home>/paused` appears
crates/tally-daemon/src/config.rs        [heartbeat] enabled defaults to true
crates/tally-daemon/tests/ensure.rs      integration tests for the three commands
.claude-plugin/marketplace.json          repo-root marketplace listing the plugin
integrations/claude-code/                the plugin (manifest, hooks, commands, vendored installers)
integrations/vscode/                     the VS Code extension
scripts/sync-extension-installers.sh     copies install/install.{sh,ps1} into both integrations
scripts/check-integrations.sh            manifest, version and hook checks run in CI
```

### Task 1: `tally ensure`

**Files:**
- Create: `crates/tally-daemon/src/ensure.rs`, `crates/tally-daemon/src/explainer.txt`, `crates/tally-daemon/tests/ensure.rs`
- Modify: `crates/tally-daemon/src/lib.rs` (`pub mod ensure;`), `crates/tally-daemon/src/main.rs` (subcommand `Ensure`, handled before the store opens like `Login`)

**Interfaces:**
- Consumes: `project::register_git_project(&mut Store, &Path, Option<&str>)`, the `run.lock` convention in `main.rs` (`Run`: `OpenOptions::create(true).write(true).truncate(false)`, `try_lock`).
- Produces:
  - `pub struct EnsureOpts { pub client: Option<String>, pub register_cwd: Option<PathBuf> }`
  - `pub struct EnsureOutcome { pub first_run: bool, pub started: bool, pub notice: Option<&'static str> }` (`notice` is the explainer, set only when `first_run`)
  - `pub fn daemon_running(home: &Path) -> bool` (true when `run.lock` is held; takes and releases the lock otherwise)
  - `pub fn ensure(home: &Path, o: &EnsureOpts, spawn: &dyn Fn(&Path) -> std::io::Result<()>) -> EnsureOutcome` (infallible: every error is appended to `<home>/ensure.log` and the outcome says what happened)
  - `pub fn spawn_daemon(home: &Path) -> std::io::Result<()>` (the real `spawn`)
  - CLI: `tally ensure [--client <name>] [--register-cwd <dir>] [--hook-json]`; always exits 0.

Behaviour of `ensure`, in order: (1) create `<home>` if missing; (2) if `<home>/paused` exists, stop here (Task 2); (3) if `consent.json` is absent, write `{"client","version","at_ms"}` and set `first_run`; (4) if `register_cwd` is a git repo, register it with no public name (local only), once; (5) unless `daemon_running`, call `spawn`. `--hook-json` prints `{"systemMessage": <explainer>}` on first run and nothing otherwise; without it nothing is printed. `spawn_daemon` starts `<current exe> run --home <home>` with stdout and stderr appended to `<home>/tally.log`, stdin null, detached from the parent (Windows `CREATE_NO_WINDOW | DETACHED_PROCESS`; unix a new process group via `CommandExt::process_group(0)`), and does not wait.

- [ ] **Step 1: Write the failing tests** in `tests/ensure.rs`, calling the library with a counting fake `spawn` and, for the CLI cases, the `tally(home, args)` helper pattern from `tests/cli.rs`:
  - `first_run_records_consent_and_starts_the_daemon`: fresh home; `ensure` returns `first_run == true`, `started == true`, spawn called once, `consent.json` parses with `client == "claude-code"`.
  - `second_run_changes_nothing`: after the first run, hold `run.lock` in the test; `ensure` returns `first_run == false`, `started == false`, spawn never called, `consent.json` bytes identical.
  - `a_failing_spawn_is_logged_not_raised`: spawn returns `Err`; `ensure` returns normally with `started == false`; `ensure.log` contains the error text; via the CLI with `--home` set to a path under a file (unwritable), `tally ensure` exits 0 and prints nothing to stderr.
  - `register_cwd_registers_an_unnamed_project_once`: a temp git repo whose path contains a space and a non-ASCII character; two runs leave `store.projects()` with one row whose public name is `None`.
  - `hook_json_is_printed_only_on_the_first_run`: CLI with `--hook-json`: first stdout parses as JSON with a non-empty `systemMessage`; second stdout is empty.
  - `concurrent_ensures_leave_one_daemon`: launch four `tally ensure` processes at once against one home with the real binary; poll up to 5 s for `daemon_running`; assert it holds and that a second `try_lock` on `run.lock` fails (only one holder); kill the daemon at the end.
- [ ] **Step 2: Run** `cargo test -p tally-daemon --features test-kit --test ensure`. Expected: FAIL (unresolved import `ensure`).
- [ ] **Step 3: Implement** `ensure.rs` as specified. Detect "already running" with the same lock file, not a pid file. Log lines are `<rfc3339 ts> <message>`; cap the file at 64 KiB by truncating to the last 32 KiB when it grows past that.
- [ ] **Step 4: Wire** the `Ensure` subcommand in `main.rs` before the store is opened; `--register-cwd` canonicalises the path (the project registry already does) and ignores a non-repo.
- [ ] **Step 5: Run** the same command. Expected: PASS, and `cargo clippy -p tally-daemon --all-targets -- -D warnings` clean.
- [ ] **Step 6: Measure.** Run `tally ensure` ten times against a home whose daemon is already running; each must finish in under 300 ms on the dev machine. Record the numbers in the PR body.
- [ ] **Step 7: Write `explainer.txt`** with the required content of plan 12 Task 7 (what Tally is for; what is captured; what is never captured; what can leave the computer, one sentence each; how to see the data, `tally status`; how to stop it, `tally stop`; how to remove it, uninstall the extension and `tally service uninstall`; who to ask), 70 columns, plain text. Edit plan 12 Task 7 so it says "reuse `crates/tally-daemon/src/explainer.txt` (plan 13 Task 1)" instead of writing its own.
- [ ] **Step 8: Commit:** `feat(daemon): tally ensure starts the daemon idempotently for extensions`.

---

### Task 2: `tally stop` and `tally start`

**Files:**
- Modify: `crates/tally-daemon/src/main.rs`, `crates/tally-daemon/src/run.rs`, `crates/tally-daemon/tests/ensure.rs`

**Interfaces:**
- Consumes: Task 1's `ensure`, `daemon_running`; `run::run_loop(store, cfg, home, stop)`.
- Produces: `tally stop` creates `<home>/paused`, waits up to 5 s for the daemon to release `run.lock`, prints `stopped` or `not running`, exits 0; `tally start` removes `<home>/paused`, runs `ensure`, prints `started` or `already running`. The run loop checks `<home>/paused` once per tick and returns cleanly (same path as Ctrl-C) when it exists.

- [ ] **Step 1: Write the failing tests:**
  - `stop_pauses_and_ensure_respects_it` in `tests/ensure.rs`: with the lock held by the test and released by a thread once `paused` appears, `tally stop` prints `stopped`; `ensure` then returns `started == false` and spawn is never called.
  - `start_clears_the_pause` in `tests/ensure.rs`: after `stop`, `tally start` removes `paused`, and the next `ensure` spawns.
  - `the_run_loop_exits_when_paused_appears` in `run.rs`: one-tick interval, a `paused` file created from the test; `run_loop` returns within 3 s.
- [ ] **Step 2: Run** the three tests. Expected: FAIL.
- [ ] **Step 3: Implement** the two subcommands and the per-tick check (`Path::exists`). `tally stop` on a home with no daemon still writes `paused` (so a later session does not start one) and prints `not running`.
- [ ] **Step 4: Run** `cargo test -p tally-daemon --features test-kit` and clippy. Expected: PASS, clean.
- [ ] **Step 5: Commit:** `feat(daemon): stop and start pause the daemon across sessions`.

---

### Task 3: Zero-config defaults and `status --brief`

**Files:**
- Modify: `crates/tally-daemon/src/config.rs`, `crates/tally-daemon/src/main.rs`, `crates/tally-daemon/tests/ensure.rs`, `docs/adapters/heartbeat.md`

**Interfaces:**
- Consumes: `HeartbeatConfig` (`enabled`, `port`, `token_file`), `status::status`.
- Produces: `[heartbeat] enabled` defaults to `true`; the daemon creates the token file at start when it is missing (`load_or_create_token`, already used by `heartbeat-config`); `tally status --brief` prints exactly one line, `Tally: <h>h <m>m verified (<n> unverified min), daemon running|stopped|paused`, built from `Status.verified_s`, `Status.unverified_s` and `daemon_running`.

- [ ] **Step 1: Write the failing tests:**
  - `heartbeat_is_on_by_default` in `config.rs` (an empty config has `heartbeat.enabled == true`; an explicit `enabled = false` still disables it).
  - `a_fresh_home_serves_heartbeats_after_ensure` in `tests/ensure.rs`: run the real `tally ensure` on an empty home; poll up to 5 s for the port from `tally heartbeat-config`; a POST without a key answers 401; with the key from `heartbeat-config` it answers 201; kill the daemon.
  - `status_brief_is_one_line` in `tests/ensure.rs`: after ingesting one 120 s claim, `tally status --brief` is a single line containing `0h 2m verified` and `daemon stopped`.
- [ ] **Step 2: Run** them. Expected: FAIL.
- [ ] **Step 3: Implement** the default, the token creation at daemon start and the `--brief` flag.
- [ ] **Step 4: Update `docs/adapters/heartbeat.md`:** setup step 1 becomes "on by default; set `enabled = false` to turn it off".
- [ ] **Step 5: Run** the suite and clippy. Expected: PASS, clean.
- [ ] **Step 6: Commit:** `feat(daemon): heartbeat server on by default, status --brief`.

---

### Task 4: Spike, what Claude Code and VS Code really do

Never invent a vendor format (plan 12 rule): Tasks 5 and 6 rely on the four facts below, each unverified until this task records it.

**Files:**
- Create: `docs/platform-notes.md` section "Extension hosts" (create the file if plan 12 Task 1 has not)

- [ ] **Step 1 (Claude Code):** In a scratch plugin directory added with `/plugin marketplace add <dir>` and installed, observe and record, with the Claude Code version: (a) a `hooks/hooks.json` `SessionStart` entry using the fields seen in superpowers 7.0.2 (`matcher` `startup|resume|clear|compact`, `command`, `powershell`, `shell`, `async`) runs on Windows and macOS/Linux; (b) hook stdout `{"systemMessage": "..."}` is shown to the user and not only to the model; (c) `${CLAUDE_PLUGIN_ROOT}` expands inside `hooks.json` commands and inside a command file's ``!`...` `` line, and `$CLAUDE_PROJECT_DIR` is set in the hook environment; (d) a process started by the hook with the Task 1 detach flags is still alive 60 s after the session ends.
- [ ] **Step 2 (VS Code):** Record that an extension with `activationEvents: ["onStartupFinished"]` can run `child_process.execFile` on Windows, and what `vscode.workspace.onDidChangeTextDocument`, `onDidSaveTextDocument`, `onDidChangeActiveTextEditor` and `onDidChangeTextEditorSelection` report for a normal file, an untitled file, a `vscode-remote:` file and an output channel (`document.uri.scheme`).
- [ ] **Step 3:** Where a fact is false, change the owning task in this plan before executing it (for example if (b) fails, the explainer goes through `additionalContext` plus a first-run notice command instead). Commit: `docs: record extension host behaviour`.

---

### Task 5: Claude Code plugin

**Files:**
- Create: `.claude-plugin/marketplace.json`, `integrations/claude-code/.claude-plugin/plugin.json`, `integrations/claude-code/hooks/hooks.json`, `integrations/claude-code/hooks/run-hook.cmd`, `integrations/claude-code/hooks/session-start`, `integrations/claude-code/commands/status.md`, `commands/stop.md`, `commands/start.md`, `scripts/check-integrations.sh`, `scripts/test-session-start.sh`
- Modify: none (the vendored installers arrive in Task 7)

**Interfaces:**
- Consumes: `tally ensure --client claude-code --register-cwd <dir> --hook-json` (Task 1), `tally status --brief` (Task 3), `tally stop`, `tally start` (Task 2), plan 12 installers `install.sh` / `install.ps1` (env `TALLY_VERSION`, `TALLY_INSTALL_DIR`, `TALLY_RELEASE_BASE`).
- Produces: `/plugin marketplace add sectersion/tally` then `/plugin install tally@tally`; slash commands `/tally:status`, `/tally:stop`, `/tally:start`; `hooks/session-start` with the contract: reads `$CLAUDE_PROJECT_DIR` and `TALLY_HOME` (default per OS: `%LOCALAPPDATA%\tally`, `~/Library/Application Support/tally`, `${XDG_DATA_HOME:-~/.local/share}/tally`), prints only hook JSON, always exits 0.

`plugin.json` carries `name` `tally`, `version` equal to the workspace version in `Cargo.toml` (currently `0.1.0`), description, repository, license. `marketplace.json` has `name` `tally`, one plugin `tally` with `source` `./integrations/claude-code`. `hooks.json` has one `SessionStart` entry, matcher `startup|resume`, `async` false, `command` and `powershell` both calling `hooks/run-hook.cmd session-start` (the polyglot runner pattern of superpowers 7.0.2: one file that is valid `cmd` and `bash`, so it works with and without Git Bash).

`session-start` behaviour: (1) locate `tally`: `<home>/bin/tally[.exe]` first, then PATH; accept it only if `tally --version` is at least the plugin's `version`; (2) if accepted, run `tally ensure --client claude-code --register-cwd "$CLAUDE_PROJECT_DIR" --hook-json` and pass its stdout through; (3) if not, start the vendored installer **detached** with `TALLY_VERSION` set to the plugin version and `TALLY_INSTALL_DIR=<home>/bin`, chained with the same `ensure` call, and print `{"systemMessage": "Tally is installing (version X); tracking starts in a minute. See <home>/ensure.log if it does not."}`; (4) any failure is appended to `<home>/ensure.log` and ends with exit 0. The script never waits for a download.

- [ ] **Step 1: Write `scripts/check-integrations.sh` first** (needs `jq`): fails unless (a) the marketplace's plugin `source` resolves to a directory containing `.claude-plugin/plugin.json`; (b) `plugin.json` `version` equals the `version` in `crates/tally-daemon/Cargo.toml`; (c) every file path named in `hooks.json` exists and is executable where it must be; (d) `integrations/vscode/package.json` `version` also equals it (skipped until Task 6 exists); (e) each command file has `description` front matter. Run it: Expected FAIL (no plugin yet).
- [ ] **Step 2: Write `scripts/test-session-start.sh`** (bash, fake `tally` first on PATH that appends its arguments to a log and answers `--version` with a chosen version). Cases, each asserting exit code 0 and valid-JSON-or-empty stdout:
  - `calls_ensure_with_the_cwd`: the log shows `ensure --client claude-code --register-cwd <dir> --hook-json`, where `<dir>` is a temp dir named `My Code é` (spaces and non-ASCII).
  - `old_binary_is_replaced_not_called`: fake `--version` says `0.0.9`; the fake installer (a script placed as the vendored `install.sh`, recording its env) runs detached with `TALLY_VERSION=0.1.0`; the old fake `tally` never sees `ensure`.
  - `offline_install_fails_quietly`: the installer exits 1; hook exit 0, stdout is the installing message only, `ensure.log` has the failure line, wall time under 5 s.
  - `unwritable_home_is_fine`: `TALLY_HOME` under a regular file; exit 0, no stderr.
  - `no_tally_and_no_installer_is_fine`: exit 0 and a `systemMessage` telling the user to reinstall the plugin.
- [ ] **Step 3: Run** `bash scripts/test-session-start.sh`. Expected: FAIL (no script).
- [ ] **Step 4: Create** the manifests, `run-hook.cmd`, `session-start` and the three command files. `status.md` has `description: Show today's verified hours and whether Tally is running`, `allowed-tools` limited to the one run-hook invocation, and a body that runs ``!`"${CLAUDE_PLUGIN_ROOT}/hooks/run-hook.cmd" tally status --brief` ``; `stop.md` and `start.md` do the same with `tally stop` / `tally start`. `run-hook.cmd` accepts `session-start` and `tally <args...>` (resolve the binary like step (1), then exec it).
- [ ] **Step 5: Run** both scripts. Expected: PASS.
- [ ] **Step 6: Install it for real** in Claude Code on the dev machine from the local repo (`/plugin marketplace add <repo path>`, `/plugin install tally@tally`), start a new session, and confirm with `tally doctor` that the daemon is running and `claude-code` events arrive. Record the result in the PR body.
- [ ] **Step 7: Commit:** `feat(plugin): Claude Code plugin that starts the daemon on session start`.

---

### Task 6: VS Code extension

**Files:**
- Create: `integrations/vscode/package.json`, `integrations/vscode/tsconfig.json`, `integrations/vscode/src/extension.ts`, `integrations/vscode/src/throttle.ts`, `integrations/vscode/src/payload.ts`, `integrations/vscode/test/throttle.test.ts`, `integrations/vscode/test/payload.test.ts`, `integrations/vscode/README.md`

**Interfaces:**
- Consumes: `tally ensure --client vscode` (Task 1), `tally heartbeat-config` (prints `api_url = ...` and `api_key = ...`), the daemon route `POST <api_url>/v1/users/current/heartbeats` with `Authorization: Basic base64(api_key)` and the `Heartbeat` fields in `crates/tally-daemon/src/heartbeat/mod.rs`.
- Produces:
  - `export type Doc = { scheme: string; fsPath: string; isUntitled: boolean }`
  - `export function isTrackable(d: Doc): boolean` (true only for `scheme === "file"` and not untitled)
  - `export class Throttle { constructor(windowMs: number); shouldSend(path: string, isWrite: boolean, nowMs: number): boolean }` (sends the first heartbeat for a path, any write, and any heartbeat at least `windowMs` after the last one for that path; `windowMs` is 120000)
  - `export function heartbeat(d: Doc, o: { isWrite: boolean; lineno: number; cursorpos: number; lines: number; language: string; nowMs: number; version: string }): object` (fields `entity`, `type: "file"`, `time` in float seconds, `is_write`, `lineno`, `cursorpos`, `lines`, `language`, `plugin: "tally-vscode/<version>"`; no file content, no branch, no project name)
- No runtime dependencies; dev dependencies only `typescript`, `@types/vscode`, `@vscode/vsce`, `bun` for tests.

Activation (`onStartupFinished`): find `tally` (`<home>/bin` then PATH); if missing, show one notification with an "Install Tally" button that runs the vendored installer (Task 7) and then `ensure`; if present, run `tally ensure --client vscode` once, and on the first run show the explainer (read from `tally ensure`'s notice via `--hook-json`) as an information message. Read `tally heartbeat-config` once per activation. Send heartbeats on save, on active-editor change and on text change, all through `Throttle`. If a POST fails, drop it silently: there is no offline queue, because the daemon discards heartbeats older than about 75 s anyway (`docs/adapters/heartbeat.md`).

- [ ] **Step 1: Write the failing tests** (`bun test` in `integrations/vscode`):
  - `isTrackable` is true for `{scheme: "file", fsPath: "C:\\My Code\\é.rs", isUntitled: false}` and false for `untitled`, `vscode-remote`, `output` and `vscode-userdata` schemes and for `isUntitled: true`.
  - `Throttle(120000)`: first send true; same path 1 s later false; same path 1 s later with `isWrite` true; same path 120 s later true; a different path at 1 s true; 10 000 calls within one window for one path send exactly once.
  - `heartbeat(...)` returns an object whose keys are exactly the allowed list above, `time` equals `nowMs / 1000`, and `plugin` is `tally-vscode/0.1.0`; a test asserts no key contains the document text by passing a long `lines`-only input.
- [ ] **Step 2: Run** `bun test` in `integrations/vscode`. Expected: FAIL.
- [ ] **Step 3: Implement** `throttle.ts` and `payload.ts` (pure, no `vscode` import) and then `extension.ts` (thin wiring; the only file that imports `vscode`).
- [ ] **Step 4: Run** `bun test`, `bunx tsc --noEmit`. Expected: PASS, no type errors.
- [ ] **Step 5: Capture the real protocol.** Package with `bunx vsce package`, install the `.vsix` with `code --install-extension`, edit and save a file in a registered repo, and run `tally doctor`: `accepted` rises, `late_dropped` and `not_a_file` stay 0. Save one request (headers and body, token removed) to `crates/tally-daemon/tests/fixtures/heartbeat/vscode-request.txt` and add a line to `NOTES.md` saying this client's requests are OBSERVED.
- [ ] **Step 6: Commit:** `feat(vscode): extension that sends throttled heartbeats to the local daemon`.

---

### Task 7: Packaging, vendored installers, CI

**Files:**
- Create: `scripts/sync-extension-installers.sh`, `.github/workflows/integrations.yml`
- Modify: `.github/workflows/release.yml` (plan 12 Task 11; add the vsix and plugin archive to the release), `integrations/claude-code/install/` and `integrations/vscode/install/` (generated copies of `install/install.sh` and `install/install.ps1`)

**Interfaces:**
- Consumes: plan 12 `install/install.sh`, `install/install.ps1`; Task 5's `scripts/check-integrations.sh`, `scripts/test-session-start.sh`; Task 6's `bun test`.
- Produces: `scripts/sync-extension-installers.sh` copies the two installers into both `install/` directories byte for byte; the workflow fails when `git diff --exit-code` shows the copies are stale.

- [ ] **Step 1: Write the failing check:** in `integrations.yml`, jobs `plugin` (ubuntu and windows: `scripts/sync-extension-installers.sh && git diff --exit-code`, `scripts/check-integrations.sh`, `scripts/test-session-start.sh`) and `vscode` (`bun install --frozen-lockfile`, `bun test`, `bunx tsc --noEmit`, `bunx vsce package`). Push the branch. Expected: FAIL until the installers exist (plan 12 Task 13).
- [ ] **Step 2: Implement** the sync script and commit the generated copies.
- [ ] **Step 3: Release:** extend `release.yml` so the draft release also attaches `tally-vscode-<version>.vsix` and lists both in `SHA256SUMS`; the version-equality check from Task 5 runs before the tag build.
- [ ] **Step 4: Run** the workflow on the branch. Expected: green on both operating systems.
- [ ] **Step 5: Commit:** `ci: build, check and release the Claude Code plugin and VS Code extension`.

---

### Task 8: Docs and dogfood acceptance

**Files:**
- Modify: `README.md` (install section), `docs/adapters/README.md`
- Create: `docs/extensions.md`

- [ ] **Step 1: Write `docs/extensions.md`:** the two install commands for Claude Code, the `.vsix` install for VS Code, what is captured (link `explainer.txt`), `/tally:status`, `/tally:stop`, `/tally:start`, how to remove everything, and the lazy-start limit from Design.
- [ ] **Step 2: Acceptance on the dev machine** (record the output in the PR body):
  1. Stop the daemon and delete the installed `tally` from PATH; remove `<home>\consent.json`.
  2. Install the plugin; open a Claude Code session in this repo. Expected: the explainer appears once, `tally doctor` shows `claude-code` ok within a minute.
  3. Quit all sessions, reboot, open a session. Expected: the daemon is running again within 10 s and no explainer is shown.
  4. `/tally:stop`, start a new session. Expected: the daemon stays stopped; `/tally:start` brings it back.
  5. Install the VS Code `.vsix`, edit a file. Expected: `tally doctor` heartbeat `accepted` rises; Claude edits do not raise it (`dropped_agent` does).
- [ ] **Step 3: Commit:** `docs: extension install guide`.

---

## Out of scope

JetBrains, Neovim and Emacs clients; macOS and Linux presence (plan 12); code signing of the `.vsix` and the plugin (plan 12 Task 12 route); the VS Code Marketplace and Open VSX listings; an OS service for always-on tracking from boot (plan 12 Task 8 to 9, unchanged); any change to scoring.
