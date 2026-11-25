# Adapters

| Harness | Status | Signals | Logs live in |
|---|---|---|---|
| claude-code | supported | prompt, interrupt, tool_decision, agent_busy, agent_output | `~/.claude/projects/**` (all OS; `%USERPROFILE%\.claude` on Windows) |
| heartbeat (editors, WakaTime protocol) | supported (protocol notes UNVERIFIED) | user_edit | no logs: plugins post to `http://127.0.0.1:<port>/api`; see [heartbeat.md](heartbeat.md). First real client: the Tally VS Code extension (`integrations/vscode`, [extensions.md](../extensions.md)); OBSERVED: a single in-editor read heartbeat; the save path is covered by unit tests only ([NOTES.md](../../crates/tally-daemon/tests/fixtures/heartbeat/NOTES.md)) |
| codex | unsupported: spike not run, needs a logged-in Codex CLI session to capture a real rollout log | none | `~/.codex/sessions/` (UNVERIFIED) |
| aider | unsupported: spike not run, needs `aider-chat` installed, a model API key and a scratch session | none | project dir `.aider.chat.history.md` (UNVERIFIED) |
| gemini-cli | unsupported: spike not run, needs a Gemini CLI install with a login and a real chat to inspect | none | `~/.gemini/tmp/<hash>/chats/` (UNVERIFIED) |
| copilot | supported for Copilot CLI (OBSERVED, v1 events); VS Code Chat unsupported (no requests ever seen); see [copilot.md](copilot.md) | prompt, agent_output, agent_busy | `~/.copilot/session-state/<id>/events.jsonl` (`%USERPROFILE%\.copilot` on Windows) |
| cursor | unsupported: spike not run, needs a Cursor install and a read of its SQLite `state.vscdb` | none | Cursor `User/workspaceStorage` (UNVERIFIED) |

No parser exists for an unsupported harness; each has a page here with what is believed, marked UNVERIFIED, and `NOTES.md` under `crates/tally-daemon/tests/fixtures/<harness>/` with the steps to run the spike.
