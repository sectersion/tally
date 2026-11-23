# Copilot spike notes (docs-only; spike NOT run)

Legend: OBSERVED = stated by vendor docs (read via fetch, not on-disk). UNVERIFIED = third-party/blog or inference. Nothing here was seen on a real machine; no fixture is committed because we will not invent data.

## Copilot CLI
- OBSERVED: default root `~/.copilot` (Windows: by extension `%USERPROFILE%\.copilot`), overridden wholesale by `COPILOT_HOME`. https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-config-dir-reference
- OBSERVED: `session-state/<session-id>/events.jsonl` (event log, powers --resume/--continue); `session-store.db` (SQLite, rebuildable); `logs/process-{timestamp}-{pid}.log`. Same URL.
- OBSERVED: docs do NOT document the events.jsonl schema or any timestamp field. Same URL.
- UNVERIFIED: each line is JSON with `type`, `timestamp`, `id`, `data`; token totals only in `session.shutdown` (`data.modelMetrics`), cumulative across resumes, absent if no clean shutdown. https://usekibble.com/blog/where-github-copilot-keeps-its-usage-logs
- UNVERIFIED: Windows `session-state` path explicitly (docs only give Windows for permissions-config.json).

## VS Code Copilot Chat
- UNVERIFIED: `<Code user dir>/User/workspaceStorage/<hash>/chatSessions/<id>.json|.jsonl` (Win `%APPDATA%\Code`, mac `~/Library/Application Support/Code`, Linux `~/.config/Code`); `.jsonl` is a mutation log, `.json` older snapshot (double-count risk); requests[] have `timestamp`, `elapsedMs`, `modelId`. Source: usekibble URL above; storage code at https://github.com/microsoft/vscode/blob/main/src/vs/workbench/contrib/chat/common/model/chatSessionStore.ts (not read in detail).

## Maintainer TODO to run the real spike
1. On a machine with Copilot CLI installed and logged in, run one short session (a prompt, a tool call, an idle gap, a clean exit), and a second resumed session.
2. Copy `~/.copilot/session-state/<id>/events.jsonl` (redact prompts/paths) to this directory; note CLI version.
3. Record: event types present, timestamp format/timezone, whether per-turn start/end exist, whether crash (kill -9) leaves a usable log, resume behaviour (same id? appended?).
4. Repeat for VS Code chatSessions (.json vs .jsonl) if VS Code support is wanted.
5. Replace every UNVERIFIED above with OBSERVED, then write the parser against the real fixture.
