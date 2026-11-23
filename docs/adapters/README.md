# Adapters

| Harness | Status | Signals | Logs live in |
|---|---|---|---|
| claude-code | supported | prompt, interrupt, tool_decision, agent_busy, agent_output | `~/.claude/projects/**` (all OS; `%USERPROFILE%\.claude` on Windows) |
| heartbeat (editors, WakaTime protocol) | supported (protocol notes UNVERIFIED) | user_edit | no logs: plugins post to `http://127.0.0.1:<port>/api`; see [heartbeat.md](heartbeat.md) |
| codex | unsupported: spike not run, needs a logged-in Codex CLI session to capture a real rollout log | none | `~/.codex/sessions/` (UNVERIFIED) |
| aider | unsupported: spike not run, needs `aider-chat` installed, a model API key and a scratch session | none | project dir `.aider.chat.history.md` (UNVERIFIED) |
| gemini-cli | unsupported: spike not run, needs a Gemini CLI install with a login and a real chat to inspect | none | `~/.gemini/tmp/<hash>/chats/` (UNVERIFIED) |
| copilot | unsupported: spike ran, but every real VS Code chat file on the spike machine has zero requests, so the request format is unseen; see [copilot.md](copilot.md) | none | VS Code `workspaceStorage/<hash>/chatSessions/*.jsonl` (container OBSERVED, requests UNVERIFIED) |
| cursor | unsupported: spike not run, needs a Cursor install and a read of its SQLite `state.vscdb` | none | Cursor `User/workspaceStorage` (UNVERIFIED) |

No parser exists for an unsupported harness; each has a page here with what is believed, marked UNVERIFIED, and `NOTES.md` under `crates/tally-daemon/tests/fixtures/<harness>/` with the steps to run the spike.
