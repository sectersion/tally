# Codex spike notes (docs-only, spike NOT run)

Status: no parser, no fixture. Nothing below is a verified log format.

## OBSERVED (this machine, read-only)
- OBSERVED: `codex --version` -> `codex-cli 0.142.4` (installed at `%APPDATA%\npm\codex`). Source: local CLI.
- OBSERVED: `codex --help` lists subcommands `exec`, `resume`, `fork`, `archive`, `delete`, `unarchive`, `doctor`, `app-server`, so sessions are persisted and addressable by id or name. Source: local CLI.
- OBSERVED: `~/.codex` (`C:\Users\Oliver\.codex`) contains only `tmp/arg0/...`. No `sessions/`, no `auth.json`, no `config.toml`, so no session has ever run here. Source: local filesystem.

## UNVERIFIED (third-party, pages not fetched or read; the official docs fetch returned HTTP 503)
- UNVERIFIED: logs live under `$CODEX_HOME` (default `~/.codex`; Windows `%USERPROFILE%\.codex`) at `sessions/YYYY/MM/DD/rollout-<timestamp>-<uuid>.jsonl`. Sources (search hits only): https://www.mantyl.dev/guides/codex-cli-session-history , https://termdeck.io/guides/where-codex-cli-stores-sessions , https://sealkeep.spala.ai/guides/where-codex-stores-sessions
- UNVERIFIED: files are JSONL, one event per line, with a first "session meta" line (cwd, id, cli version). Sources: https://yasyf.github.io/cc-transcript/docs/guide/codex-rollouts.html , https://www.bough.run/articles/what-is-in-a-codex-rollout-file
- UNVERIFIED: a `history.jsonl` of prompts may exist in `~/.codex`. No source checked.
- UNVERIFIED: the schema is not a stable public contract and may change between CLI versions (0.142.4 is much newer than most guides).
- Official docs to read: https://github.com/openai/codex (docs/config.md, `codex-rs/` rollout recorder source) and https://developers.openai.com/codex

## Maintainer steps to run the real spike
1. Install/confirm codex, run `codex login` (needs a ChatGPT or API account).
2. In a scratch git repo, run a short `codex exec "say hi"` and one interactive session that edits a file; also one `codex resume`.
3. `ls -R ~/.codex` (and `$CODEX_HOME` if set); record paths per OS (Windows, macOS, Linux).
4. Copy the produced rollout JSONL, with secrets, paths and prompts scrubbed, into this directory as the fixture.
5. Record per line: event type names, timestamp field and format, session id, cwd, model, token usage, user/assistant/tool markers, and how session end or abort looks.
6. Check: are timestamps per event? Is there an explicit session end? What does resume do (new file or append)? Are files written live (tail-able) or at exit? Are files rewritten or rotated?
7. Replace each UNVERIFIED above with OBSERVED plus the file/line evidence; only then write the parser and tests.
