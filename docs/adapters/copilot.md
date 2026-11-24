# GitHub Copilot adapter

**Status: supported for the Copilot CLI / agent host** (`session-state/<id>/events.jsonl`, seen with Copilot CLI 1.0.111, event format version 1). **VS Code Copilot Chat is not supported**: every chat file on the spike machine had zero requests, so its request format is unseen. Evidence and every OBSERVED/UNVERIFIED item: `crates/tally-daemon/tests/fixtures/copilot/NOTES.md`.

## Where logs live

| Surface | Path | Seen on disk |
|---|---|---|
| Copilot CLI, Windows | `%USERPROFILE%\.copilot\session-state\<session-id>\events.jsonl` | yes |
| Copilot CLI, macOS / Linux | `~/.copilot/session-state/<session-id>/events.jsonl` | no (docs only, UNVERIFIED) |
| Override | `COPILOT_HOME` env var, or `[copilot] home = "<dir>"` in `config.toml` | `COPILOT_HOME` UNVERIFIED |
| VS Code Chat (not parsed) | `%APPDATA%\Code\User\workspaceStorage\<hash>\chatSessions\*.jsonl` | yes, but empty of requests |

Config (`<home>/config.toml`): `[copilot]` with `enabled` (default true) and `home`. Only sessions modified in the last 7 days are read. `tally doctor` shows the files found, the last event time, the format (`cli-events-v1`) and the error rate. Prompt text is never stored: only lengths and derived numbers leave the parser.

## Honesty table

| Signal | Status | How |
|---|---|---|
| prompt | supported | `user.message` with `delivery == "idle"`; the typed `content` only, never the injected context |
| agent_output | supported | chars of non-empty `assistant.message` content |
| agent_busy | supported | prompt time to the last `assistant.turn_end` before the `agentStop` hook (or next prompt / shutdown) |
| project attribution | supported | `session.start` `context.cwd` through the git root |
| interrupt | unsupported | no abort/Stop was ever recorded in the sample |
| tool_decision | unsupported | the only decisions seen were automatic (`host_policy`), not the user's |
| agent writes (heartbeat filter) | unsupported | only read tools (`view`, `glob`, `rg`) were seen; write tool names unknown |

Messages typed while the agent is running (`delivery` other than `idle`) are not counted as prompts until their format is verified, so a busy session can under-count. A session killed mid-turn leaves an open turn that is not emitted and stops holding the clock after 6 hours.

## To extend

Run a CLI session with an edit, an approved and a rejected command, a message typed mid-turn, a Stop, and a kill; follow the maintainer TODO in `NOTES.md`.
