# Copilot spike notes

Spike run 2026-11-23 on the plan author's Windows machine by reading the files VS Code already wrote (VS Code 1.147.0; no new session was produced).

Legend: OBSERVED = seen in a real file on this machine (evidence given). UNVERIFIED = vendor docs, third-party write-ups or memory. Nothing UNVERIFIED is used by code.

## Result: no parser was written (plan Task 11 Step 4 stop rule)

Every real chat session file on this machine holds ZERO requests: 64 session files (57 `.jsonl`, 7 `.json`), all with `"requests": []`. The shape of a request (prompt text, timestamps, response, tool calls, cancellation) has therefore never been seen, and we do not invent it. Task 12 (parser and source) is blocked until a session that contains at least two prompts exists. See "Maintainer TODO".

## VS Code Copilot Chat storage (container format)

- OBSERVED: path `%APPDATA%\Code\User\workspaceStorage\<32-hex workspace hash>\chatSessions\<session-uuid>.jsonl` (57 files) and `...\<session-uuid>.json` (7 files, older mtimes 2026-01 to 2026-04, so legacy snapshots; the newest `.json` is 2026-04-04). Only `Code` exists under `%APPDATA%` (no Insiders, no VSCodium, no Cursor).
- OBSERVED: `.jsonl` is an append-only mutation log, one JSON object per line, top-level keys only `kind` and `v`, plus `k` on patch lines (88 lines in 57 files, 0 unparsable).
  - `kind: 0` = initial snapshot, always the first line, `v` is the whole session object. 57 of 57 files.
  - `kind: 1` = set a value: `k` is a path array (only `["responderUsername"]` and `["inputState"]` seen, 31 lines), `v` the new value.
  - UNVERIFIED: further kinds (array push/splice for appending a request, delete) exist in VS Code's mutation log; none appear in any local file, so `kind` values other than 0 and 1 must be counted as skipped.
- OBSERVED: snapshot `v` keys (jsonl): `version` (always `3`), `creationDate` (epoch ms), `initialLocation` (`"panel"`), `responderUsername` (`"GitHub Copilot"` or `""`), `sessionId` (uuid, equals the file name), `hasPendingEdits`, `requests` (array, empty everywhere), `pendingRequests` (array), `inputState` (object: `attachments`, `mode {id, kind}` = `agent`, `selectedModel`, `inputText`, `selections`, `permissionLevel` = `"default"`, `contrib`). One file lacks `inputState` entirely.
- OBSERVED: legacy `.json` is a single object with the same shape plus `lastMessageDate` and sometimes `responderAvatarIconUri`; `version` is `3`; no `pendingRequests`.
- OBSERVED: files are appended to (jsonl) rather than rewritten, by the file layout; the append behaviour on a live request was not watched. Whether a `.json` and a `.jsonl` can describe the same session was not checked (UNVERIFIED; dedupe by session id).
- OBSERVED: the per-file `sessionId` and the `selectedModel` metadata carry no prompt text. `accountLabel` in `selectedModel.metadata.auth` is the signed-in account name (scrubbed in the fixture).
- NOT OBSERVED: any request object, so also: per-request timestamp, response duration, how a human prompt differs from an agent follow-up, tool call accept/reject, Stop/cancel. Every one of these is UNVERIFIED. Third-party write-ups (https://usekibble.com/blog/where-github-copilot-keeps-its-usage-logs) say `requests[]` entries carry `timestamp`, `elapsedMs`, `modelId`; unconfirmed here.
- OBSERVED (indirect): one session did run an agent edit (a `chatEditingSessions/<session>/state.json` of 4861 bytes with `requestId` fields and an edit operation exists next to a `.jsonl` whose snapshot still has `requests: []`). So requests can run without being persisted in `chatSessions`; the chat file may only be written on a clean flush. Treat this adapter as possibly under-counting even once the format is known.
- OBSERVED: `chatEditingSessions/<session>/state.json` (version 2: `timeline.checkpoints[].requestId`, `operations[]` with `uri`, `requestId`, `epoch`, `edits`) records agent file edits, not prompts. Not used.

## Workspace mapping

- OBSERVED: `workspaceStorage/<hash>/workspace.json` exists in 61 of 62 dirs. Shapes seen: `{"folder": "file:///c%3A/..."}` (58; lowercase drive letter, `%3A` for the colon), `{"workspace": "file:///..."}` (1; a multi-root `.code-workspace` file, not a folder), `{"folder": "vscode-vfs://github/<owner>/<repo>"}` (2; virtual/remote, no local root). One dir has no `workspace.json`.
- Mapping rule (decided from these observations): `folder` with a `file://` URI decodes to a project root; `workspace`, `vscode-vfs://` and anything else maps to no project.

## Copilot CLI

- OBSERVED: `%USERPROFILE%\.copilot` exists with `config.json` (131 bytes, a managed-settings comment header only), `ide/`, and `logs/process-<ms>-<pid>.log` (text lines `<ISO time> [LEVEL] [component] message`, no chat content). There is NO `session-state/` directory and no `events.jsonl`: no CLI chat has ever been run here.
- UNVERIFIED (docs): `session-state/<session-id>/events.jsonl`, `COPILOT_HOME` override, https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-config-dir-reference. Event schema is not documented there.
- UNVERIFIED (third party): events have `type`, `timestamp`, `id`, `data`; token totals only in `session.shutdown`. Not built on.

## Fixtures (scrubbed from the real files above)

- `1.147.0/chat-session.jsonl`: a real session file (snapshot plus three `responderUsername` patches), strings replaced by `x` runs of equal length, enum-like values kept, sessionId and `creationDate` replaced by fixed fakes. Contains no request.
- `1.147.0/legacy-chat-session.json`: same treatment for a real legacy `.json`.
- `1.147.0/workspace.json`: the observed `folder` shape with a fake path.
- `1.147.0/drift/chat-session.jsonl`: SYNTHETIC. The snapshot with `requests` renamed to `turns`, one `kind:1` line truncated mid-JSON, one line without `kind`. (The plan's "one request missing its timestamp" cannot be made: no request has been observed.)

## Maintainer TODO to unblock Task 12

1. In VS Code with Copilot Chat signed in, open a scratch git repo, send two prompts in agent mode, accept one tool call, reject one, press Stop during one response, wait a minute, then CLOSE the window cleanly (flush).
2. Read the newest `chatSessions/<id>.jsonl` of that workspace (find its hash by `workspace.json`). Expect `kind` 2 or similar lines that append to `requests`. Record every request field (names only), the timestamp field and unit, response completion marker, tool accept/reject and cancel markers, and how an agent follow-up request differs from a typed prompt.
3. Replace each NOT OBSERVED above with OBSERVED, scrub into `1.147.0/` (or the new version dir), and then write the parser.
