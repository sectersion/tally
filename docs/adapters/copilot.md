# GitHub Copilot adapter

**Status: unsupported, spike not run.** tally cannot track Copilot time yet. We have not seen real Copilot logs, so no parser exists. See `crates/tally-daemon/tests/fixtures/copilot/NOTES.md`.

## Where logs are believed to live (all UNVERIFIED on disk)

| Surface | Path |
|---|---|
| Copilot CLI (all OS) | `~/.copilot/session-state/<session-id>/events.jsonl` (`COPILOT_HOME` overrides root; Windows `%USERPROFILE%\.copilot\...`) |
| VS Code Chat, Windows | `%APPDATA%\Code\User\workspaceStorage\<hash>\chatSessions\` |
| VS Code Chat, macOS | `~/Library/Application Support/Code/User/workspaceStorage/<hash>/chatSessions/` |
| VS Code Chat, Linux | `~/.config/Code/User/workspaceStorage/<hash>/chatSessions/` |

The CLI root and `events.jsonl` name come from GitHub docs; the VS Code paths and all file formats come from third-party write-ups.

## Expected signals (honesty table)

| Signal | Expectation | Confidence |
|---|---|---|
| Session start/end | Likely from event timestamps | UNVERIFIED |
| Active time per turn | CLI: unknown; VS Code: `elapsedMs` per request maybe | UNVERIFIED |
| Idle gaps | Inferable only if events are timestamped | UNVERIFIED |
| Token usage | CLI: only at clean shutdown, cumulative; VS Code: per request | UNVERIFIED |
| Crash/kill | Probably no shutdown event, totals lost | UNVERIFIED |
| Project attribution | Session cwd likely recorded; VS Code via workspace.json | UNVERIFIED |
