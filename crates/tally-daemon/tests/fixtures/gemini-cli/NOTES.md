# gemini-cli spike notes

Status: DOCS-ONLY. The real tool was not installed (`gemini` not on PATH, no `~/.gemini` on the spike machine), not logged in, and not run. No fixture exists on purpose: do not invent one.

## Claims
- UNVERIFIED (docs): sessions live in `~/.gemini/tmp/<project_hash>/chats/`, hash derived from project root. Source: https://geminicli.com/docs/cli/session-management/
- UNVERIFIED (docs): sessions record prompts, model responses, tool inputs/outputs, token usage (input/output/cached), and thoughts when available. Field names are not documented. Same source.
- UNVERIFIED (docs): session UUID is passed to `--resume`; unknown whether it appears in file names. Same source.
- UNVERIFIED (docs): retention via `general.sessionRetention` (`maxAge` default 30d, `minRetention` 1d). Same source.
- UNVERIFIED: file format (JSON vs JSONL), per-OS paths on Windows/macOS (assumed `%USERPROFILE%\.gemini\...` and `~/.gemini/...`), whether `GEMINI_CLI_HOME` or similar overrides it.
- UNVERIFIED: checkpointing (https://google-gemini.github.io/gemini-cli/docs/cli/checkpointing.html) and OpenTelemetry (https://google-gemini.github.io/gemini-cli/docs/cli/telemetry.html) are alternative signal sources; not evaluated.

## Maintainer steps to run the real spike
1. Install gemini-cli and log in (or set GEMINI_API_KEY).
2. Run a short session in a throwaway git repo: one prompt, one tool call, one resume (`--resume`), then exit; repeat with a second project dir.
3. `find ~/.gemini -newer <marker> -type f`; record exact paths, names, extension.
4. Capture file(s) verbatim; scrub secrets only; save as fixtures here.
5. Record: session id location, per-message timestamps (present? timezone?), role/tool markers, token fields, whether files are appended or rewritten, project path recoverability from `<project_hash>` (which hash? verify against project root).
6. Verify on Windows and macOS/Linux; check env/settings overrides of the base dir.
7. Replace every UNVERIFIED above with OBSERVED, then write the parser against the fixtures.
