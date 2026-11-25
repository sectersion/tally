## StruggleTime (derek aydin): weak, mostly dead code

Rust workspace + Python `server.py` (~2.4k lines), ~23.7k lines total, AGPL-3.0, one commit. **Not an LLM judge and not a log parser.** Read: all of `st-core` and `st-daemon`, most of `st-helper/adapters.rs`, the Claude Code and shell hooks, batch-ingest (`server.py` ~2085-2235), calibration and privacy docs. Not read: `st-tray`, `st-server`, `src/verify.py`, docs-site, the Godot/Unity/JetBrains/neovim adapters, CI.

**Capture.** `st-daemon/src/lib.rs` polls the project tree every 1 s (mtime and size per file), and drains `~/.aitime/agent_events.jsonl` and `exec.jsonl`. The Claude Code hook just `printf`s a fixed JSON line on PostToolUse/Stop. It never reads `~/.claude/projects`. No Cursor/Aider/Copilot/Gemini logs, no OS idle, no git (`commits` is hardcoded 0).

**"Struggle".**
- Agent vs human is a file-change burst heuristic: 5+ files in a second, or 3+ files over 8,000 bytes, counts as AI. A formatter or `git checkout` looks like an agent. Agent time is never subtracted.
- `verify.rs::weight()` is a first-match state machine (`AI_STRUGGLE`/`STRUGGLE_LOOP`/`CLOSED_LOOP` 1.0, `AI_REVIEWED` 0.85, `HUMAN_UNVERIFIED` 0.70, `EXEC_ONLY` 0.50, `AI_UNREVIEWED` 0.25, `OPEN_LOOP_AGENT_DUMP` 0.20).
- `is_ai_struggle()` depends on counters (`same_file_reedit_count`, `ai_retry_loops`, `test_fail_to_pass_transitions`, `agent_turn_churn`) that the Rust daemon doesn't produce, so most of it is dead code. What actually gives full weight is `exec_failures>0 && executions>1 && (human_edit||commit)`: a failed command, another command, then any file change.
- The "20/20 calibration" is author-labelled synthetic fixtures run against the same rules. Circular.

**Trust is weak.**
- Unkeyed SHA-256 chain, and `ensure_sync_shape` rewrites and re-hashes old entries.
- Per-install Ed25519 signing exists only in Python. The Rust daemon sends `install_id: "rust-daemon"` unsigned, and the server accepts and weights unsigned buckets.
- The server trusts client-sent `active_secs`.
- Anyone can append to `agent_events.jsonl`. Dedup is on identical lines, so a changing field defeats it.

**Privacy.** Good: `FORBIDDEN_KEYS` (`st-core/src/agent_events.rs:8`) is rejected recursively in daemon and server, and file ids are `sha256(salt:path)[..16]`. Sloppy: `fetch_adapters_from_github` downloads and installs the adapters from an unpinned `refs/heads/main`, the shell hook logs the git subcommand, docs hardcode a domain that isn't the live host, evidence is retained after account deletion.

**License:** AGPL-3.0, same as ours, so code reuse is legal with attribution. Nothing here is worth porting anyway.

## Verdicts
| Item | Verdict |
|---|---|
| Shared `FORBIDDEN_KEYS` check at the daemon *and* server boundary, recursive | **steal**. Check we reject unknown fields on the server, not only omit them in the daemon |
| Cap activity ticks per drain so a retry storm can't inflate time | **steal** (verify our per-action cap covers it) |
| Fail, another run, then an edit as a cheap struggle signal | **adapt**, only from real harness events, never from file mtime |
| Silent-streak decay across buckets | **avoid**. Per-bucket decay makes weights depend on bucket alignment |
| File-burst size as the agent/human split | **avoid** |
| Heartbeats counted as human edits | **avoid** |
| Unkeyed chain, unsigned buckets accepted, trusting client `active_secs` | **avoid** |
| Self-labelled synthetic calibration | **avoid**. Calibrate on real labelled sessions |
