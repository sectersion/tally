# GitHub Copilot adapter

**Status: unsupported. The spike ran, but no real chat request exists on the spike machine, so no parser was written.** tally cannot track Copilot time yet. Details and evidence: `crates/tally-daemon/tests/fixtures/copilot/NOTES.md`.

## Where logs live

| Surface | Path | Seen on disk |
|---|---|---|
| VS Code Chat, Windows | `%APPDATA%\Code\User\workspaceStorage\<hash>\chatSessions\<session-id>.jsonl` | yes (VS Code 1.147.0) |
| VS Code Chat, macOS | `~/Library/Application Support/Code/User/workspaceStorage/<hash>/chatSessions/` | no (UNVERIFIED) |
| VS Code Chat, Linux | `~/.config/Code/User/workspaceStorage/<hash>/chatSessions/` | no (UNVERIFIED) |
| Workspace to project | `workspaceStorage/<hash>/workspace.json`, `{"folder": "file:///c%3A/..."}` | yes; `workspace` and `vscode-vfs://` forms map to no project |
| Copilot CLI | `~/.copilot/session-state/<id>/events.jsonl` (docs only) | no: `~/.copilot` has only `config.json`, `ide/` and process logs |

A `.jsonl` chat file is an append-only mutation log (`kind` 0 = whole-session snapshot, `kind` 1 = set a path). Older sessions are a single `.json` snapshot. All 64 files here contain an empty `requests` array, so the shape of a prompt, a response, a tool decision or a cancellation is not known.

## Honesty table

| Signal | Status | Why |
|---|---|---|
| prompt | unsupported | no request ever seen; cannot tell typed prompts from agent follow-ups |
| agent_output | unsupported | no response ever seen |
| agent_busy | unsupported | no request or response timestamps seen |
| interrupt | unsupported | cancellation never seen |
| tool_decision | unsupported | accept/reject never seen |
| agent writes (heartbeat filter) | unsupported | `chatEditingSessions/*/state.json` records agent edits but is not parsed |
| project attribution | known | `workspace.json` folder URI, once prompts are parsed |

Also noted: one session ran an agent edit while its chat file still shows no requests, so even a finished adapter may under-count until VS Code flushes.

## To unblock

Send two prompts in agent mode (accept one tool call, reject one, press Stop once), close the window cleanly, and follow the maintainer TODO in `NOTES.md`.
