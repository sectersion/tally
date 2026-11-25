## Sablier (Alice): the strongest entry so far

Go, AGPL-3.0 (same as ours, so porting with attribution is fine), ~25k lines, many pure-function tests. Read at `bbab5da`. I spot-checked the constants and flags below against the source. Not read: `launch.go`, `sync.go`, `launcher/relay.go`, the opencode mapper, `update.go`, the SQL.

**Capture is hook-based, not log-based.** `sablier claude|codex|opencode` wraps the harness in a PTY and injects hooks (`harness/claude/settings.go` merges into `--settings`, Codex uses `-c hooks.X=[...]`, opencode runs `opencode serve` on localhost). It never parses `~/.claude/projects/*.jsonl`. Anything started outside the wrapper is invisible, and there is no Cursor, Aider, Copilot, Gemini, git or WakaTime. Tally's log-parsing approach covers more.

**Credit model** (`internal/credit/credit.go`, `Version = 3`, `AgentCap` 5 min, `InputCap` 2 min):
- Each prompt, approval or interrupt earns up to 5 min after it, plus a window before it back to the last turn end, capped at 5 min.
- Typing, mouse and scroll ticks each earn a 5 s window plus 2 min after.
- Leaving the terminal for over 2 min cuts credit from the moment you left.
- A sweep-line (`attribute()`) means overlapping time counts once. Observed input beats inferred windows.
- Flat cap per action, no substance check. One keystroke per 2 min earns continuous credit. This is the gap Tally's substance caps close.

**Input sensing (`launcher/input.go`).** Counts raw stdin in 5 s windows: key chunks, SGR/X10 mouse, bracketed paste (`\e[200~`) and focus in/out (`\e[I` / `\e[O`). Stores counts only. This is the same idea as Hindsight's `typed_permille`, done without an editor extension.

**Permission dialogs are inferred.** `PermissionRequest` followed by a matching `PostToolUse` gives approval plus its latency. A denial is synthesized from the next prompt or `Stop`. The pairing is "last PreToolUse per agent", which is fragile, but approval latency is a real human-time signal we don't use.

**Trust.**
- `event/chain.go` is sha256 over length-prefixed fields (prev, seq, ts ms, kind, session, project, harness, source, data).
- `server/ingest.go` verifies hashes and seq/prev, treats an identical re-send as a duplicate and a different one as a 409, and rejects timestamps over 5 min ahead.
- `server/flags.go` marks events late, stale (over 24 h), backfill (days) or unchained, and a reviewer sees them.
- The chain is unkeyed, so a forged chain from seq 1 verifies. It only detects edits after upload. Ours has the same limit unless the server witness closes it, which is worth stating in our threat model.
- `sablier verify` has the server sample prompts, the client uploads the text, and an LLM labels each as iteration or delegation. There is an HMAC of the prompt so uploaded text matches what was sent. This is opt-in, but the text goes to a third-party model.

**Privacy.** Metadata, project id (git remote or folder name), install UUID and prompt HMACs leave. Prompt text stays in local sqlite. `install.sh` verifies a sha256 from the same origin as the binary, so it protects against corruption and not tampering, and `SABLIER_RELEASE_URL` can redirect the download.

## Verdicts
| Item | Verdict |
|---|---|
| Bracketed-paste / focus-escape input counting | **steal** (optional wrapper mode) |
| Approval latency as human time | **steal** |
| Pure `Map(state, hook) -> (state, events)` adapters, tested without I/O | **steal** (test style) |
| "Don't guess when the lower bound is unknown" | **steal** |
| `Version` constant on the scoring rules | already have (ruleset), keep |
| Late / stale / backfill flags on the server | **steal** |
| Sweep-line overlap resolution | **adapt**, check against our claim-since-previous-action |
| Hook wrapper as a capture source | **adapt**, optional next to log parsing. Hook-injected `--settings` can clash with user settings |
| Flat 5 min after-window | **avoid** (pays for reading time) |
| Unended turn treated as busy forever | **avoid** |
| LLM verify with prompt upload | **avoid** (breaks "no text leaves"). Our self-hosted judge is better |
| `install.sh` | **adapt**, add signature verification |
