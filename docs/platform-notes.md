# Platform notes

Facts about vendor tools, observed on a real machine. Never invent a vendor format: a fact is either OBSERVED (with the tool version and the command), NOT OBSERVED (with why) or UNVERIFIED.

## Extension hosts

Observed 2026-11-24 on Windows 11 (10.0.26300), Claude Code 2.1.331, VS Code 1.147.0, Node 24.12 (plan 13 Task 4). Probes used scratch plugins and extensions in the system temp dir; all were removed and the temp dirs deleted (a first deletion of `%TEMP%\tally-spike-4\proj` failed with "busy", most likely because one of my own earlier shells had it as its cwd; a read-only process listing showed only `bash.exe`/`powershell.exe` command lines, holder not pinpointed, nothing killed; the directory deleted fine later).

### Claude Code 2.1.331

Scratch marketplace + plugin on disk, driven by headless `claude -p` (stream-json with `--include-hook-events`) in a scratch git dir.

| Item | Result |
|---|---|
| (a) `hooks/hooks.json` `SessionStart` with `matcher` `startup\|resume\|clear\|compact`, `command`, `powershell`, `shell`, `async` | OBSERVED on Windows: the hook ran on `startup` and on `resume` (`claude -p --continue`). With `"shell":"bash"` and with no `shell` field, the **`command`** string ran, and the default shell on Windows was Git Bash (a hook `echo` showed `BASH_VERSION=5.2.37`, `$0=/usr/bin/bash`, `%COMSPEC%` unexpanded, `$PSVersionTable` empty, via `claude --plugin-dir`); behaviour on a machine without Git Bash: NOT OBSERVED, UNVERIFIED. The `powershell` string was not run. With `"shell":"powershell"` the `command` string ran under PowerShell (`& node ...` worked). `async:true` entries also ran. Stdin is JSON `{session_id, transcript_path, cwd, hook_event_name:"SessionStart", source}` where `source` is `startup` or `resume`. macOS and Linux: NOT OBSERVED (no such machine), UNVERIFIED. `clear` and `compact` matchers: NOT OBSERVED (not exercised in headless mode). |
| (b) stdout `{"systemMessage":"..."}` | OBSERVED: arrives in the `hook_response` event (`stdout`, exit 0, outcome `success`). The model, when asked in a second session whether the text was in its context, said it did not see it (self-report, not proof). Whether the USER sees it: NOT OBSERVED (needs a human at the terminal: install a plugin whose hook prints a systemMessage, start `claude`, look for the line under the session banner). |
| (c) `${CLAUDE_PLUGIN_ROOT}` | OBSERVED expanding inside `hooks.json` commands (both shells) and inside a command file's `` !`echo ROOT=${CLAUDE_PLUGIN_ROOT}` `` line (output `ROOT=C:/Users/.../plug`, forward slashes). With a local-directory marketplace the root is the source folder (`readFromFolder`), not a copy in `~/.claude/plugins/cache`. Also set in the hook env: `CLAUDE_PROJECT_DIR` (forward slashes `C:/Users/...` under bash, backslashes under `shell:"powershell"`), `CLAUDE_PLUGIN_DATA`, `CLAUDE_ENV_FILE`, `CLAUDE_CODE_SESSION_ID`. Invoking a `.cmd` file from a command file's `!` line: NOT OBSERVED (only `echo` was tried). |
| (d) detached child survives the session | OBSERVED: a `node` hook did `spawn("ping",["-n","120","127.0.0.1"],{detached:true,windowsHide:true,stdio:"ignore"})` (libuv: `DETACHED_PROCESS`, plus `CREATE_NO_WINDOW` from `windowsHide`) and `unref()`. The child was still alive 90 s after `claude -p` exited, and the hook returned without waiting for it. Only the recorded ping PIDs were stopped. Rust `CommandExt::creation_flags` was not exercised; the flag values are the same. macOS/Linux process-group behaviour: UNVERIFIED. |
| (e) non-interactive plugin commands | OBSERVED, all exit 0 with stdin `/dev/null` and no TTY, no `-y` needed for a plain marketplace: `claude plugin marketplace add <dir>` (only a local-directory marketplace was tried; the `owner/repo` and `url` forms are UNVERIFIED, the sectersion/tally marketplace is not published yet); `claude plugin install <plugin>@<marketplace>` (default `--scope user`); `claude plugin list` and `claude plugin list --json` (array of objects with `id` such as `name@marketplace`, `version`, `scope`, `enabled`); `claude plugin uninstall <plugin>@<marketplace>`; `claude plugin marketplace remove <name>` (also uninstalls its plugins). Idempotent: a second `marketplace add` and a second `install` print "already" and exit 0. Uninstalling a plugin that is not installed exits 1. Installing from a marketplace that is not added exits 1. `claude plugin marketplace remove <name>` for a marketplace that is not present exits 1 ("not found"), so Task 7's remove must check `claude plugin marketplace list` first. Also available: `claude plugin install --marketplace <source> <plugin>` (adds the marketplace first). |

### VS Code 1.147.0

Scratch extension (`activationEvents:["onStartupFinished"]`) packaged with `npx @vscode/vsce package`, installed with `--user-data-dir` and `--extensions-dir` pointing at temp dirs, then a window launched with the same dirs; the extension scripted the edits itself.

- `code` on Windows is `...\Microsoft VS Code\bin\code` (a shell script) and `code.cmd` (cmd): run it through `cmd /C code` from Rust.
- `code --install-extension <file.vsix> --force` OBSERVED: exit 0, prints "Extension 'x.vsix' was successfully installed." (also on a second run, so it is idempotent). `code --list-extensions` prints `publisher.name` per line; `--show-versions` adds `@version`. Both print a harmless Node `DEP0169` deprecation warning on stderr.
- `onStartupFinished` activation OBSERVED; `child_process.execFile("cmd.exe",["/c","echo","execFile-ok"])` returned `execFile-ok`, no error. `process.platform` was `win32`.
- Events, with `document.uri.scheme` / `isUntitled`:

| Document | `onDidChangeActiveTextEditor` | `onDidChangeTextDocument` | `onDidSaveTextDocument` | `onDidChangeTextEditorSelection` |
|---|---|---|---|---|
| normal file (`file`, path with spaces and an accent) | fires, `file` | fires, `file`, `contentChanges.length` 1 for a real edit, 0 for dirty-flag changes (several extra events with 0) | fires, `file` | fires, `file` |
| untitled (`untitled`, `isUntitled` true, fsPath `Untitled-1`) | fires (a `null` editor event first), `untitled` | fires, `untitled` | NOT OBSERVED (a save opens a dialog) | fires, `untitled` |
| output channel (`output`, language `Log`) | fires, `output` | fires on `appendLine`, `output`, 1 change | NOT OBSERVED (cannot be saved) | NOT OBSERVED |
| `vscode-remote:` | NOT OBSERVED: no remote host on this machine; `openTextDocument(vscode-remote://...)` fails with "Unable to resolve resource", so the scheme string is UNVERIFIED | NOT OBSERVED | NOT OBSERVED | NOT OBSERVED |

  `workspace.textDocuments` also held a `chatSessionInput` scheme document. So `scheme === "file"` and not untitled is the right filter; every other scheme seen is excluded by it.
- Cursor, Windsurf, Antigravity, VSCodium, Trae: not installed (no CLI on PATH), not tested. `--install-extension` / `--list-extensions` on those: UNVERIFIED.

## Idle time per OS (plan 12 Task 1)

Probe: `cargo run -p tally-daemon --example idle_spike` (prints `idle_ms` once a second through `default_platform()`).

### Windows 11 (10.0.26300), `GetLastInputInfo` minus `GetTickCount64`, milliseconds

- OBSERVED 2026-11-24: untouched, the value grew 250, 1250, 2250 ms one second apart; after input it reset to 31 ms. Matches the call's documented meaning.
- Across a lid-close or hibernate sleep: NOT OBSERVED (needs a human closing the lid for 2 minutes). UNVERIFIED whether the counter resets, keeps counting or jumps. Plan 04's gap rule does not depend on which.

### macOS (Apple silicon / Intel), Ubuntu X11, Fedora GNOME Wayland, Sway (ext-idle-notify-v1)

NOT VERIFIED: no such machine available to the executor. Later tasks (3 to 6) must keep their manual checklists open for each of these; their sleep behaviour and the presence of `ext-idle-notify-v1` / `org.gnome.Mutter.IdleMonitor` are unknown.

### Linux/X11 (plan 12 Task 4)

OBSERVED 2026-11-24 in a `rust:1` Docker container under `xvfb-run`: `X11Platform` (XScreenSaver `QueryInfo`) returns `ms_since_user_input` and the value is at least 1000 after a 1.2 s sleep with no input. NOT VERIFIED on a real X11 desktop (typing resets the counter): needs a human.

### Linux Wayland (plan 12 Task 5)

`MutterPlatform` (zbus, `GetIdletime`) and `ExtIdlePlatform` (ext-idle-notify-v1, 30 s timeout) compile and pass clippy `-D warnings` in a `rust:1` container. NOT VERIFIED on a real GNOME Wayland session, on Sway, or on a compositor with neither (needs `tally status` to print the unsupported line and `tally run` to refuse): no such desktop available. The container has no D-Bus or Wayland, so only `available() == false` paths ran.

### Lid-close checklist (plan 12 Task 6)

Automated: `finalize::tests::sleep_gap_holds_for_every_idle_timer_wake_behaviour` covers an idle timer that resets, keeps counting or jumps on wake (no OS needed).
Manual, per OS (Windows, macOS, Linux/X11, Linux/GNOME-Wayland): run `tally run`, work 1 minute, close the lid 10 minutes, open, send one prompt; then `sqlite3 <home>/tally.db "select * from coverage"` must show a gap over the sleep and `tally status` must count the prompt under `skipped_claims` (`no_presence_coverage`). Windows hibernate and Linux suspend-to-RAM too if available.
Status: NOT VERIFIED on all four (needs a human with the hardware).
