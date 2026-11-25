# ccusage: log-format drift early warning

Cloned `--depth 1` to `<scratch>/ccusage/src` (HEAD dated 2026-11-25, 1 commit, read only, nothing run). The tree is a Rust rewrite (`rust/adapters/*`); the TypeScript app is a shim.

## Summary
ccusage reads about 20 harnesses, including Claude Code, Codex and Gemini, but not Cursor. Its source comments carry dated, issue-numbered NOTE lines. That makes them a ready-made drift timeline, because git history is not in a shallow clone. Its Claude dedupe is far more elaborate than `message.id + requestId`, and it keys on message and request, never on `uuid`. That supports dropping the session id from Tally's event id. License is MIT.

## Findings

1. **Harnesses and paths.** The adapters under `rust/adapters/` are amp, antigravity, claude, cline, codebuff, codex, copilot, droid, dsh, gemini, goose, grok, hermes, kilo, kimi, openclaw, opencode, pi, qwen and zcode. No Cursor adapter.
   - Claude (`crates/ccusage-cli/src/home.rs::claude_config_dirs`): `CLAUDE_CONFIG_DIR` is a comma-separated list. It expands `~`, de-duplicates by canonical path, and accepts a path ending in `projects` (#2107). The default is `$XDG_CONFIG_HOME|~/.config/claude` plus `~/.claude`. Home comes from HOME, then USERPROFILE, then HOMEDRIVE+HOMEPATH, so Windows is covered. `adapters/claude/src/paths.rs::claude_paths` keeps only dirs containing `projects/`. It errors if the env var is set and none are valid. The walk is recursive and handles `projects/{p}/{session}.jsonl` and `projects/{p}/{session}/subagents/*.jsonl` (`extract_session_parts`). Overlapping roots attribute a file to the deepest root (#2374).
   - Codex (`adapters/codex/src/paths.rs::codex_home_paths`): `CODEX_HOME` is a comma list, default `~/.codex`. It reads `sessions/` and `archived_sessions/`, and falls back to the dir itself for saved `codex exec --json` output. It reads both `.jsonl` and `.jsonl.zst`. If both exist, plain wins. If a rollout is in both `sessions/` and `archived_sessions/`, `sessions/` wins (#1890).
   - Gemini: `GEMINI_DATA_DIR` list, default `~/.gemini/tmp` (`adapters/gemini/src/paths.rs`).

2. **Fields and validation.** There is no valibot or zod. Validation is serde in Rust.
   - Claude (`claude/src/daily/record.rs::RawUsageRecord`): `timestamp` and `message.usage` are required, and a line without them is skipped. `usage.input_tokens` and `usage.output_tokens` are required. `cache_*` tokens, `speed`, `cache_creation.ephemeral_5m/1h`, `iterations` and `costUSD` are optional. Also optional: `version`, `sessionId`, `requestId`, `isSidechain`, `isApiErrorMessage`, `message.id` and `message.model`. A `null` in a guarded field returns `Rejection::NullContract`, which is skipped quietly. It also reads `data.message.message` in AgentProgress records (#977). Post-checks in `is_valid_usage_record`: `version` must look like semver, and the ids and model must not be empty strings. It never reads `type`; any line with usage counts. Unknown fields are ignored.
   - Codex (`types/wire.rs`, `parser.rs`): it matches `session_meta`, `turn_context`, `event_msg` with `payload.type=token_count`, `compacted`, `token_usage_record` and `thread_settings_applied`. It reads `info.total_token_usage` and `last_token_usage`, and it accepts alias keys (`prompt_tokens`, `completion_tokens`, `cached_tokens`, `reasoning_*`). Timestamps may be strings or numbers, with fallbacks to `created_at` and `createdAt`.

3. **Dedupe** (`claude/src/dedupe.rs`, `daily/response_chain*`).
   - Key: hash of `message.id` + `requestId`. When `requestId` is missing, the key also includes the effective session and the timestamp (#1799).
   - Copies of one response across different transcripts or sessions match by message and request id across sessions (#1765). That is the forked/resumed case.
   - `/btw` sidechains replay parent usage with the same message id and a new requestId. They are matched across request ids and the parent wins (#913, #2255).
   - Blocks of one response are written one per line with different timestamps. Without a requestId they are joined by walking the `uuid`/`parentUuid` chain, with a byte-level scanner as a fast path (#1838, #1935, #2255).
   - Forks: matching is by fork depth, and each depth counts once (#2208, #2366). This is the closest answer to Tally's open question.
   - Tie-break: prefer the copy with fast speed, a cache split or a recorded cost (#886).
   - Session id in `ClaudeEntry` is `recorded_session_id` (the `sessionId` inside the line), falling back to the path. A transcript can record a different session than its directory (#1794).
   - **Verdict on Tally:** ccusage never keys on `uuid`. Its key is message id + request id, and the session is added only when requestId is absent. So Tally's `claude:{session}:{uuid}:{suffix}` will double count a forked or resumed copy, because the session differs. This partly settles the question: do not put the session in the id. UNVERIFIED: whether `uuid` itself is preserved when Claude Code copies history into a resumed file. ccusage's chain code is "keyed by effective session as well as uuid since a copied transcript reuses them", which implies copies do reuse uuids. If so, `claude:{uuid}:{suffix}` would dedupe resumed copies. For sidechain replays, `uuid` may differ, so those need a message-id rule.
   - Compaction and streaming: Claude has no special compaction handling. Codex handles compaction through `compacted` records and advancing `token_count` (#1821). Streaming partials are handled only as the multi-block-per-response case above, and as a truncated final line, which is parsed fully and rejected if unclosed.

4. **Drift timeline** (from dated NOTEs, since the clone has no history).
   - 2026-03-09 #886: fast-mode `speed` field.
   - 2026-05-19 #977: usage nested in AgentProgress `data.message.message`.
   - 2026-05-25 #913: `/btw` sidechain replays.
   - 2026-07-08 (CC 2.1.119) #1405: `effort` omitted.
   - 2026-07-10 #1423: advisor usage reported separately.
   - 2026-07-26 (Codex 0.144.0) #1438: `thread_settings_applied` without `service_tier`; "standard" vs "default" spellings.
   - 2026-07-27 #1499: Codex output includes reasoning; totals missing or zero in exec JSON.
   - 2026-09-16 (CC 2.1.268) #1719: `model` can be null from 2.1.266.
   - 2026-09-18 #1765: a response copied into several transcripts.
   - 2026-09-25 #1794: transcript session id differs from its directory.
   - 2026-10-02 #1821: Codex compaction ordering.
   - 2026-10-14 #1890: Codex `.zst` rollouts.
   - 2026-11-09 (Codex VS Code 0.164.0-alpha) #2051: subagents drop `token_usage_record`.
   - 2026-11-16: synthetic error, refusal and interrupt records with null speed and zero tokens (#2085 advisor `cache_creation` split).
   - 2026-11-18 #2097: Codex rollout path dates are in local time.
   - 2026-11-20 #2208, #2255, #2366, #2376: fork and requestless dedupe, mtime restore.

5. **Time and I/O.**
   - Dates are bucketed with `jiff` in `--timezone` or the system zone (`date_utils.rs::resolve_tz`). Logged timestamps are UTC ISO. Codex also accepts epoch numbers; below 1e10 they are read as seconds (`common/timestamps.rs`).
   - `--since` prunes by mtime with a one-day margin, but a file is skipped only if the last timestamp in its 16 KB tail is also old (#2376), because restored files keep stale mtimes. For Unix it also uses the later of ctime and mtime (#2531). Related files of one session are kept together via union-find on recorded session ids.
   - Reading is line-buffered with a recycled buffer (128 KiB doubling, capped at 1 MiB), a memchr prefilter before JSON parse, parallel workers capped at 16 (#2476), and a walker thread. Lines are mmap-free `read_until`.

6. **License.** MIT (`LICENSE`, Copyright 2025 ryoppippi). It is permissive and GPL-compatible, so code can be copied into an AGPL-3.0 project if the MIT notice and copyright are kept. Tests and fixtures can be copied with attribution. Pure observation of format facts needs nothing.

## Recommendation
- Copy as tests (re-expressed as Tally fixtures):
  - Duplicate response across two session files.
  - `/btw` sidechain replay.
  - Per-content-block lines with differing timestamps.
  - Null `model` or `usage`.
  - Synthetic error line with zero tokens.
  - AgentProgress nested message.
  - Truncated final line.
  - Restored file with an old mtime.
  - `CLAUDE_CONFIG_DIR` as a comma list or pointing at `projects/`.
  - Codex `.zst`, plus plain and archived copies.
- Guard now: nullable `model`, `speed`, `version`, `requestId`; sessionId inside the line differing from the filename; subagents under `{session}/subagents/`; both default config dirs; an id that is stable across forks, using `uuid` plus kind without the session, after checking a real resumed file.
- Subscribe to ccusage releases (or watch the NOTE lines with `git log -S"NOTE("`). It is a cheap and fast drift alarm: Claude Code changes showed up there within days (versions 2.1.119 and 2.1.268 were named).

## Not verified
- Git history, release notes and issues (shallow clone; the issue links are UNVERIFIED and unopened).
- Whether Claude Code reuses `uuid` in resumed or forked files (inferred only).
- Codex fork-depth and replay-filter internals (`replay.rs`, not read in full).
- Cline, Copilot, Cursor-adjacent adapters, `jsonl.rs` internals and benchmark claims (the blog's numbers were not re-measured).
- Dates look auto-generated in the future relative to common releases; version labels are quoted as written.
