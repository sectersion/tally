# Editor and agent extensions

Tally gets its signals from two small extensions: a Claude Code plugin and a VS Code extension (which also loads in VS Code forks). `tally setup` is meant to install both. This page says what exists today and how to use it.

## Status

- **Built:** `tally setup` (detect, install, remove, dry run), `tally ensure`, `tally stop`, `tally start`, `tally status --brief`, the Claude Code plugin (SessionStart hook plus `/tally:status`, `/tally:stop`, `/tally:start`), the VS Code extension, and the heartbeat server (on by default, 127.0.0.1 only).
- **Planned, not built:** the one-line installers (`install.sh`, `install.ps1`), release archives with the `.vsix` inside, a published `sectersion/tally` plugin marketplace, listings on the VS Code Marketplace or Open VSX, and an OS service that starts the daemon at boot (plan 12). There is no release yet and no download URL.
- **Not done:** the acceptance run for the one-liner (stop everything, run the installer, open a session and a window, reboot, `/tally:stop`, run it again with `--dry-run` and see `already installed`). It cannot be run until the installers exist.
- **Verified** (see [platform-notes.md](platform-notes.md); Windows 11, Claude Code 2.1.331, VS Code 1.147.0): the plugin commands, hook behaviour under Git Bash, a detached child surviving the session, `code --install-extension` and `--list-extensions`, and which VS Code events fire. A single in-editor read heartbeat from the extension reached the daemon ([notes](../crates/tally-daemon/tests/fixtures/heartbeat/NOTES.md)); the save path is covered by unit tests only.
- **UNVERIFIED:** macOS and Linux hook behaviour and process detach, the Cursor family (`--install-extension` on Cursor, Windsurf, VSCodium, Antigravity, Trae), `vscode-remote:` documents, how `systemMessage` renders to the user, the `claude plugin marketplace add owner/repo` form, and a machine without Git Bash.

## Install today

Nothing here downloads anything. You need a clone of this repo and a Rust toolchain; for the extension also `bun` and Node.

1. Build `tally` and put it on PATH:

   ```
   cargo install --path crates/tally-daemon
   ```

2. Claude Code plugin, from the clone (the forms observed in platform-notes.md for a local directory):

   ```
   claude plugin marketplace add <path to your clone>
   claude plugin install tally@tally
   ```

   The hook only starts a `tally` it finds in `<tally home>/bin` or on PATH. Its download branch needs an installer that does not exist yet, so without `tally` on PATH it prints a message and does nothing.

3. VS Code extension, from source (this build and package was run; the install of a scratch `.vsix` is in platform-notes.md):

   ```
   cd integrations/vscode
   bun install
   bunx tsc -p .
   bunx vsce package --out <dir>/tally-vscode.vsix
   ```

   Then either `code --install-extension <dir>/tally-vscode.vsix` or `tally setup --only vscode --extras-dir <dir>` (setup looks for the file name `tally-vscode.vsix`).

4. Check first with `tally setup --dry-run`. Add `--yes` to skip the questions; without a terminal setup refuses unless `--yes` is given.

Do not rely on a bare `tally setup` for Claude Code today: it runs `claude plugin marketplace add sectersion/tally`, and that marketplace is not published (UNVERIFIED form). Use step 2 and `--only vscode`.

## What `tally setup` does

It prints the explainer, lists what it found, asks `Continue? [y/N]` once (skipped with `--yes` or `--dry-run`), installs, then starts the daemon (which records your consent), sends one test heartbeat to `127.0.0.1`, and prints `daemon: running`, `paused (run tally start)` or `not running`.

Integrations, in the order setup checks them:

| Name | Detected by | Installed by |
|---|---|---|
| VS Code | `code` on PATH, then the usual install folder; only a `~/.vscode` folder is reported `config only` and not installed | `code --install-extension <extras-dir>/tally-vscode.vsix --force`; already in `--list-extensions` means already installed |
| Cursor | `cursor`, config folder `~/.cursor` | same, UNVERIFIED |
| Windsurf | `windsurf`, `~/.windsurf` | same, UNVERIFIED |
| Antigravity | `antigravity`, `~/.antigravity` | same, UNVERIFIED |
| VSCodium | `codium`, `~/.vscode-oss` | same, UNVERIFIED |
| Trae | `trae`, `~/.trae` | same, UNVERIFIED |
| Claude Code | `claude` on PATH; only a `~/.claude` folder is `config only` | `claude plugin marketplace add sectersion/tally`, then `claude plugin install tally@tally`; already in `claude plugin list --json` means already installed |
| JetBrains (only with `--include-wakatime`) | `idea`, `pycharm`, `webstorm`, `goland`, `clion`, `rubymine`, `phpstorm`, `rider`, `rustrover`, `datagrip` | the stock WakaTime plugin, plus `~/.wakatime.cfg` (below). Not exercised by me. |

Flags (from `tally setup --help`):

- `--dry-run`: print what would be done; run and write nothing. Real output on this machine, against a scratch home (paths shortened):

  ```
  VS Code: found (<install folder>\bin\code.cmd)
  Cursor: not found
  Windsurf: not found
  Antigravity: not found
  VSCodium: not found
  Trae: not found
  Claude Code: found (<home>\.local\bin\claude.exe)
    VS Code: Skipped("would install")
    Claude Code: Skipped("would install")
  ```

  A dry run only detects; it does not say `already installed`. With `--remove` it says `would remove`.
- `--yes`: do not ask anything.
- `--only <name>`: only these integrations (`vscode`, `cursor`, `claude-code`); repeatable. Case and punctuation are ignored.
- `--remove`: uninstall the extensions. The data directory is never touched.
- `--extras-dir <dir>`: where `tally-vscode.vsix` is (default: an `extras` folder next to the `tally` program).
- `--include-wakatime`, `--force`: see the last section.

Only `--dry-run` was run for real. The install and remove paths are covered by unit tests with a fake environment, not run against real editors by me.

## Slash commands

In Claude Code, with the plugin installed: `/tally:status` runs `tally status --brief` (verified time, unverified minutes, whether the daemon runs), `/tally:stop` runs `tally stop`, `/tally:start` runs `tally start`.

`tally stop` stops the daemon and writes a `paused` marker, so `tally ensure` (which the plugin and the extension call) does not start it again in later sessions. `tally start` removes the marker and starts the daemon.

## What is captured

The text setup shows you is [explainer.txt](../crates/tally-daemon/src/explainer.txt): counts and lengths of your actions, never the words; nothing leaves the machine until you link a project, publish, or run `tally submit`. The heartbeat server listens on 127.0.0.1 only and needs a token the daemon creates. The VS Code extension sends file path, line, cursor, line count and language, at most one per file every 2 minutes plus one per save, to `127.0.0.1` only. See [adapters/heartbeat.md](adapters/heartbeat.md).

The explainer's "Remove it" line mentions `tally service uninstall`; there is no `service` command yet (plan 12). Use the steps below.

## Limit: the daemon starts lazily

The daemon is started by the first Claude Code session or VS Code window, not at boot. Time before that is not seen. An OS service that runs from boot is plan 12 and is not built.

## Remove everything

```
tally stop
tally setup --remove
```

`--remove` uninstalls the VS Code-family extension and the Claude Code plugin and marketplace. It never deletes your data: delete the tally home folder yourself if you want it gone. By hand: `code --uninstall-extension sectersion.tally`, `claude plugin uninstall tally@tally`, `claude plugin marketplace remove tally`. `cargo uninstall tally-daemon` removes the binary from step 1 (not run). `~/.wakatime.cfg` is left alone.

## `~/.wakatime.cfg`

Only `--include-wakatime` touches it, and only after a JetBrains product was found and you consented. It writes `api_url` and `api_key` under `[settings]` and keeps every other line and section. If the file already has a different `api_url` or `api_key`, it reports a conflict and changes nothing, unless `--force`: then the old file is kept as `.wakatime.cfg.tally-bak` first. Either way a stock plugin sends to one place only, so pointing it at Tally stops it sending to wakatime.com or Hackatime. `--remove` never edits the file. To do it by hand, run `tally heartbeat-config` (see [adapters/heartbeat.md](adapters/heartbeat.md)).
