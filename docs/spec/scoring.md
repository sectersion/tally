# Tally Scoring Spec (draft v0)

The contract every component implements: the daemon computes it, the server re-computes it (same Rust crate compiled to WebAssembly), auditors re-derive it. If code and this spec disagree, the spec wins until it is changed.

Ruleset version: `2026.11-0`. Every constant below is part of the ruleset; changing any value bumps the version.

## 1. Concepts

- **Event:** something observed on the machine (a prompt, a test run, an idle span).
- **Human action:** an event that represents a decision by the user. Only human actions claim time.
- **Claim:** seconds a human action earns, split into `verified_s` and `unverified_s`.
- **Ledger entry:** one claim, hash-chained, uploaded. For logged hours, ledger entries, head hashes with commit timestamps (sync) and badge data and public ledger files (publish) are all that leave the machine; see the README privacy section.
- **Project:** a git repository root. Local id = SHA-256 of the canonical absolute root path; server id assigned on link.

## 2. Event schema

All events share:

| Field | Type | Notes |
|---|---|---|
| `v` | int | Schema version, starts at 1 |
| `id` | string | Unique string. Adapters derive it from the source log (for example `claude:<session>:<entry>:prompt`) so re-reading a log yields the same ids. |
| `ts` | int | Unix milliseconds, UTC |
| `source` | string | Adapter id: `claude-code`, `codex`, `heartbeat`, `git`, `shell`, `presence` |
| `project` | string \| null | Local project id |
| `session` | string \| null | Harness session id, when there is one |
| `kind` | string | One of the kinds below |

Kinds and their extra fields:

| Kind | Human action? | Extra fields |
|---|---|---|
| `prompt` | yes | `chars`, `relevance_pm` (0–1000), `novelty_bytes` |
| `interrupt` | yes | none |
| `tool_decision` | yes | `decision`: `approve` \| `deny` |
| `user_edit` | yes | `file_hash`, `lines_changed` |
| `test_run` | yes | `cmd_hash`, `exit_code`, `initiator`: `user` \| `agent` |
| `commit` | yes, if `by = user` | `sha`, `by`: `user` \| `agent`, `lines_changed` |
| `agent_busy` | no | `end_ts` (span) |
| `idle` | no | `end_ts` (span where OS idle time exceeded `IDLE_SAMPLE_MIN`) |
| `agent_output` | no | `chars` (used for the reading allowance) |

Prompt text, file contents, and commands never appear in events. Only hashes and derived numbers do.

`test_run` with `initiator = agent` is not a human action but feeds fail→pass detection.

## 3. Hours algorithm

Input: all events for one user, sorted by `ts`, across all projects.

For each human action `a` at time `t`, with the previous human action (any project) at `t_prev`:

1. **Interval.** `I = [max(t_prev, t − MAX_INTERVAL), t]`. A run starts at the first action ever and at any action more than `MAX_INTERVAL` after the previous human action; for the first action of a run, `I = [t − FIRST_ACTION_S, t]`. Time away longer than `MAX_INTERVAL` is never credited, even when the machine produced no idle samples (asleep, daemon off).
2. **No reading allowance.** `agent_output` events never protect time from idle or agent-busy subtraction, so a stretch away stays bounded (section 9.1) however much the agent writes.
3. **Thinking allowance.** If `a` is a substantive prompt (`relevance_pm ≥ THINK_RELEVANCE_PM`), the last `THINK_MAX_S` seconds before `t` are also protected. Deep thinking counts only when it ends in a substantive decision.
4. **Idle subtraction.** For each `idle` span inside `I`, subtract the part beyond the first `IDLE_GRACE_S` seconds, excluding protected windows.
5. **Agent-busy subtraction.** For each `agent_busy` span inside `I`, subtract the overlap with idle time beyond `AGENT_IDLE_S`.
6. **Raw seconds** = what remains of `I`.
7. **Cap.** `verified_s = min(raw, cap(a))`, `unverified_s = raw − verified_s`.
8. **Attribution.** The claim belongs to `a.project`.

Credited hours = sum of `verified_s`. Unverified seconds are shown to reviewers and never credited automatically.

**Every project is reviewed by a human before it ships.** Tally's hours are what the reviewer starts from, not a final verdict. Reviewers may approve unverified seconds or remove verified ones; each change is recorded with the reviewer's id and a reason.

## 4. Caps

| Action | Cap |
|---|---|
| `prompt`, trivial (`chars < 20` or `relevance_pm < 150`) | 90 s |
| `prompt`, otherwise | `120 + relevance_pm × 1.08` s (max 1200 s) |
| `interrupt` followed by a `prompt` within 120 s | 120 s |
| `interrupt`, otherwise | 30 s |
| `tool_decision = approve` | 60 s |
| `tool_decision = deny` | 180 s |
| `user_edit` | 180 s |
| `test_run`, user-initiated | 120 s |
| `test_run` completing a fail→pass loop | 1200 s |
| `commit`, `lines_changed < 3` | 60 s |
| `commit`, otherwise | 600 s |

**Fail→pass loop:** a passing `test_run` whose `cmd_hash` matches an earlier failing run within `LOOP_WINDOW_S`, with at least one human action in between. A loop completes once: if a passing run of the same `cmd_hash` falls between the failing run and this one, this run is an ordinary `test_run`.

## 5. Relevance and novelty

Computed locally on prompt text, which is then discarded.

- **Tokens:** lowercase words matching `[a-z_][a-z0-9_]{2,}`, minus a fixed stopword list.
- **Context vocabulary:** tokens from all git-tracked text files in the project, including comments, README, and other Markdown, capped at `VOCAB_MAX` tokens (most recently modified files first); plus the last agent output and the last error output. Including prose lets plain-language prompts ("the jump feels floaty") match the project's own words.
- `relevance_pm = 1000 × |prompt ∩ context| / max(|prompt|, 1)`.
- `novelty_bytes = len(zstd(context_tail ‖ prompt)) − len(zstd(context_tail))`, level 3, where `context_tail` is the last 8 KiB of conversation.
- If `novelty_bytes < NOVELTY_MIN`, `relevance_pm` is set to 0 (the prompt adds nothing).

## 6. Ledger entry (uploaded)

```json
{
  "v": 1,
  "ruleset": "2026.11-0",
  "action_id": "01J...",
  "ts": 1763800000000,
  "project": "<public project name, e.g. sectersion/tally>",
  "kind": "prompt",
  "verified_s": 412,
  "unverified_s": 0,
  "reasons": ["idle_subtracted", "cap_prompt"],
  "features": { "relevance_pm": 270, "novelty_bytes": 96 },
  "prev_hash": "<hex>",
  "hash": "<hex>"
}
```

The `project` field is the public project name, set before chaining, and is never rewritten; the server keeps its own project id separately.

`hash = SHA-256(canonical JSON of the entry without "hash")`. Canonical JSON: sorted keys, no whitespace, integers only. The daemon commits the latest hash to the server every `COMMIT_INTERVAL_S`.

## 7. Constants

| Name | Value |
|---|---|
| `MAX_INTERVAL` | 3600 s |
| `FIRST_ACTION_S` | 120 s |
| `IDLE_SAMPLE_MIN` | 30 s |
| `IDLE_GRACE_S` | 180 s |
| `THINK_MAX_S` | 300 s |
| `THINK_RELEVANCE_PM` | 500 |
| `AGENT_IDLE_S` | 45 s |
| `LOOP_WINDOW_S` | 3600 s |
| `NOVELTY_MIN` | 24 bytes |
| `VOCAB_MAX` | 50,000 tokens |
| `COMMIT_INTERVAL_S` | 300 s |

All values are starting points, to be calibrated against human-labeled sessions.

## 8. Deferred (not in ruleset 2026.11-0)

- Surviving-diff check for `user_edit` (M2).
- "Failing test existed before the loop" check (M2).
- Judge labels at submission and their effect on claims (M3).
- Repo-delta cap (M3).

## 9. Decisions

1. **Silent thinking:** idle time beyond `IDLE_GRACE_S` is subtracted, except a thinking allowance of up to `THINK_MAX_S` before a substantive prompt. Thinking that leads nowhere isn't counted; a stretch away from the keyboard can earn at most 5 minutes, and only when followed by a substantive prompt.
2. **Plain-language prompts:** project vocabulary includes comments and prose files, and the trivial threshold is 150‰.
3. **Unverified seconds:** never credited automatically. Every project gets a human review before it ships.
4. **Event ids:** stable derived strings, not ULIDs, so a re-read log is deduplicated by id. Nothing orders by id except as the tie-break in (ts, id).
