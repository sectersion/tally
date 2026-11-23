# Claude Code session-log structure (Claude Code 2.1.x)

Measured 2026-11-22 with a structure-only script (key paths, value kinds, counts, enum-like values) over 11 project dirs:
about 70 main session files, about 135 subagent files, about 19k uuid-bearing entries. Counts below are totals over those dirs.
Nothing here is copied from a log value. The fixture under `projects/__PROJECT__/` is hand-written to this shape.

## Deviations from the plan

1. **Interrupted turns almost never get a `turn_duration`.** Of about 43 interrupts, 1 was followed by a
   `turn_duration`; the other turns end with the interrupt entry and the next prompt (or nothing). A session
   state machine that only closes a span on `turn_duration` must also close it on an interrupt entry (and, as a
   fallback, on the next turn start). The fixture's P2 turn deliberately shows the rare "interrupt then
   `turn_duration`" order; the rule is: close at the first of interrupt / `turn_duration` / next turn start, and
   ignore a `turn_duration` that arrives with no open turn (never observed, 0 of 499).
2. **Turns are started by task notifications too.** `turn_duration` follows `origin.kind: task-notification`
   entries (own turn, own `turn_duration`, `promptSource: system`) as well as human prompts. 499 of 499
   `turn_duration` entries closed an open turn started by a human or task-notification prompt. If only human
   prompts are counted as turn starts, the task-notification turn durations look like extra ones. These are not
   human time.
3. **Human classification has a rare fallback.** 365 of 368 human prompts have `origin.kind: "human"`; 3 have no
   `origin` but `turnOrigin: "human"` (string content, no `promptSource`). Rule: human if `origin.kind == "human"`,
   or `origin` is absent and `turnOrigin == "human"`. SDK turns (`turnOrigin: "sdk"`, `promptSource: "sdk"`, no
   `origin`, 49 seen) are not human.
4. `promptSource` values seen: `typed`, `queued`, `suggestion_accepted`, `system` (task notifications), `sdk`, or absent.
5. The brief's "interrupt inside a `tool_result`" variant was not seen (0); see Q2.

No contradiction was found with: unique uuids, human tool decision = `permissionDecision.source != "config"`,
`file_path` key names, interrupt top-level shape.

## Entry types

Top-level `type` seen: `user`, `assistant`, `system`, `attachment`, `queue-operation`, `file-history-snapshot`,
`file-history-delta`, `ai-title`, `atis-latch`, `last-prompt`, `mode`, `permission-mode`, `api-request`,
`api-request-blob`, `api-request-shape`, `pr-link`, `worktree-state`, `relocated`, `cost-state`, `tools-used`.
Only `user`, `assistant`, `system` matter; everything else (and any unknown type) is ignored.

Common keys on `user`/`assistant`/`system` (and `attachment`): `uuid`, `parentUuid` (string, or null on the first
entry), `timestamp`, `sessionId`, `isSidechain`, `cwd`, `gitBranch`, `userType`, `entrypoint`, `version`. Some
entries also carry a second `session_id` key; use `sessionId`. Subagent entries add `agentId`.

- Human prompt (`user`): `origin: {kind: "human"}`, `turnOrigin: "human"`, `promptSource`, `promptId`,
  `message: {role: "user", content: <string>}`. Content is a string for every prompt seen (368 of 368), never an array.
- Task notification (`user`): `origin: {kind: "task-notification"}` (188 main-file entries; plus 9 subagent entries
  with `isMeta: true`), `turnOrigin: "task_notification"`, `promptSource: "system"`, string content.
  `origin.kind: "coordinator"` (8, subagent files, `isMeta: true`, no `turnOrigin`) is not a prompt.
  `origin` may also carry `producer` and `runId`.
- Tool result (`user`): `message.content` is an array of `{type: "tool_result", tool_use_id, content, is_error?}`
  (`is_error` present on some, absent on others), plus top-level `toolUseResult`
  (object, or a string for a few), `sourceToolAssistantUUID`, and `permissionDecision`.
- Assistant: `message: {id, role, model, content[], stop_reason, usage, ...}`. `content[]` items are `text`,
  `thinking`, `tool_use` (`id`, `name`, `input`, `type`). One API message (same `message.id`) is split over
  several entries (one per content block); this happens in most files. Each entry has its own unique `uuid`. `stop_reason` is `tool_use`, `end_turn`, `max_tokens`,
  `stop_sequence` (or null/absent on non-final blocks).
- Tool input key names (counts of the dominant shapes): `Bash` and `PowerShell`: `command` (+ `description`,
  `timeout`, `run_in_background`); `Edit`: `file_path`, `old_string`, `new_string`, `replace_all`; `Write`:
  `file_path`, `content`; `Read`: `file_path` (+ `offset`, `limit`); `Grep`: `pattern`, `path`, `output_mode`,
  ...; `Glob`: `pattern`, `path`. A few `Write`/`PowerShell` entries have only `__unparsedToolInput` (no
  `file_path`/`command`): treat a missing key as "unknown tool input", not an error.
- `system`: `subtype` values seen: `turn_duration` (`durationMs`, `messageCount`, optional
  `pendingBackgroundAgentCount`, `isMeta: false`), `away_summary`, `stop_hook_summary`. Others per the recon notes
  (`local_command`, `compact_boundary`, `informational`) were not seen in these dirs.
- Subagent files: `<session>/subagents/agent-*.jsonl`; every entry `isSidechain: true`, with `agentId`, `sessionId`
  of the parent session. Main-session files always have `isSidechain: false`.

## Answers to the seven questions

1. **`permissionDecision.source` values**: `config` (2735 of 2737) and `user_temporary` (2). `decision` is `accept`
   (2733) or `reject` (4, all `source: config`). `reasonType`: `mode` (2685), `classifier` (50), absent (2, both
   `user_temporary`). A `user_temporary` decision also carries `grantScope`; a `reasonCode` key appeared once.
   Human-produced: `user_temporary`. `config` decisions (mode / classifier / rule) are automatic. No other
   source value was seen, so rule: human decision = `source` present and `!= "config"`.
2. **Interrupts**: 43 top-level interrupt entries, shape: `type: user`, `message.content` an array with one
   `{type: "text", text: "[Request interrupted by user..."}` item. Neither `origin` nor `turnOrigin` nor
   `promptSource` is present (0 of 43). 22 are exactly `[Request interrupted by user]`; the other 21 start with the
   same prefix and have a suffix (match on prefix `[Request interrupted by user`). The `tool_result`-nested
   variant from the recon was seen 0 times in these logs; handle it by also checking `tool_result.content` text
   if cheap, but do not rely on it.
3. **`turn_duration`**: `timestamp - durationMs` is within 1 s of the turn's starting prompt (human or
   task-notification) for 478 of 499 (96%); the other 21 are off by more than 1 s (15 start more than 60 s before the prompt, 6 more than 1 s after it); cause unknown. Counting turn
   starts as human prompts only, `turn_duration` entries exceed human prompts because task-notification turns
   get their own. Exactly one per started turn (0 observed with no open turn). For interrupted turns it is
   written in 1 of about 43 cases (see Deviations). Turns still open at end of file (no `turn_duration`, not
   interrupted) are common: about 18 of the human-prompt turns were followed by another prompt with no
   `turn_duration` and no interrupt.
4. **`uuid` uniqueness**: unique within every file (0 duplicates among about 19k uuid-bearing entries) and unique across all files of
   a project dir, main and subagent (0 shared among about 18k uuids). No resumed-session history copies were seen.
   Unique event ids per `(sessionId, uuid)` are safe; do not assume a copied history cannot occur.
5. **Rewrites / truncation**: only appends were observed. Three dirs were scanned twice minutes apart while a
   session was writing: 0 files shrank, 0 first-4KB changes. Behaviour over a day, across `/clear` and
   compaction, is **unknown** (no multi-day snapshots). `relocated` entries exist (relocated cwd), so treat a file
   whose size shrinks or whose head changes as a reset, and re-read it.
6. **Partial last line**: 0 of about 205 files lacked a trailing `\n`, 0 lines failed JSON or UTF-8, including a
   live file. A snapshot read cannot prove a partial write never happens (the writer appends whole lines in
   practice), so this is **unknown** from structure alone. The fixture includes a partial last line;
   the reader must not consume bytes after the last `\n`.
7. **Timestamps**: all 100% `YYYY-MM-DDTHH:MM:SS.mmmZ` (UTC, milliseconds, `Z`); 0 exceptions. Note `queue-operation`,
   `api-request*`, `pr-link`, `file-history-delta` also carry `timestamp`; others (`ai-title`, `last-prompt`, `mode`,
   ...) carry none. Still parse defensively (RFC 3339).

## Fixture

`projects/__PROJECT__/` stands for a project dir; `__CWD__` is the literal `cwd` value in every entry (tests
substitute a temp git repo); `__PROJECT__` is a placeholder directory name. All ids are invented
(`aaaaaaaa-0000-4000-8000-0000000000NN` sessions, `bbbbbbbb-...`/`cccccccc-...`/`dddddddd-...` entry uuids,
`msg_fixture_N`, `toolu_fixture_N`, agent `a000000000000001`). Lines are `\n`-terminated; the files must not be
rewritten by git (see `.gitattributes`, `-text`).

`aaaaaaaa-0000-4000-8000-000000000001.jsonl` (all times `2026-11-22T10:..Z`), line by line:

| line | time | content |
|---|---|---|
| 1-2 | 10:00:00.000 / .100 | `attachment`, `queue-operation` (ignored) |
| 3 | 10:00:05.000 | human prompt P1, 189 chars |
| 4-5 | 10:00:08 / :10 | assistant text (600 chars) then `Bash` `cargo test -p demo` (`toolu_fixture_1`, same `message.id`) |
| 6 | 10:00:30 | tool result, `is_error: true`, decision accept `user_temporary` (human) |
| 7-8 | 10:00:35 / :40 | `Bash` `git commit -m wip`, result with `source: config`, `reasonType: mode` |
| 9-10 | 10:01:00 / 10:01:05 | assistant text, `turn_duration` 60000 ms (closes P1, start = P1 timestamp) |
| 11-13 | 10:02:00 .. :10 | task-notification user entry, assistant text, its `turn_duration` 10000 ms |
| 14-17 | 10:03:00 .. :21 | human prompt P2, assistant text, interrupt entry, `turn_duration` 21000 ms |
| 18 | n/a | malformed JSON `{"type":"user","message":` |
| 19 | n/a | line containing invalid UTF-8 byte `0xff` |
| 20 | 10:05:00.000 | human prompt P3, last complete line, no `turn_duration` (open turn) |
| 21 | 10:05:03 | partial first half of an assistant entry, **no trailing `\n`** |

Line numbers are 1-based over `\n`-split lines; lines 18 and 19 are the two bad lines. The attachment and
queue-operation lines are numbered 1 and 2 above, so entries use uuid suffixes 1..17 (queue-operation has no uuid).

`aaaaaaaa-0000-4000-8000-000000000002.jsonl`: one human prompt at 10:00:20.000 (inside session 1's P1 turn
10:00:05 to 10:01:05), an assistant text, `turn_duration` 30000 ms at 10:00:50.

`aaaaaaaa-0000-4000-8000-000000000001/subagents/agent-a000000000000001.jsonl`: `isSidechain: true`, `agentId`,
shaped like a human prompt, a `Bash` `cargo test -p demo`, and a tool result with `source: user_temporary`.
All of it must be ignored (sidechain).

P1 words that tests can place in a demo repo `README.md`: **demo**, **widget**, **parser**, **ledger**
(P1 also uses `readme`). P3 and the other prompts reuse `demo`, `widget`, `parser`, `ledger`.

The fixture sets no `permissionDecision.grantScope`, `promptSource` for the sidechain entries beyond `typed`, or
`reasonCode`; later tasks should not read those keys.
