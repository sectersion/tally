## Implementation writeups for every "steal"

I read Tally's code for each item and measured the log-related ones against my own `~/.claude/projects` (structure-only scripts that print counts and lengths, never text, in `datamine/scripts/`). **That changed several verdicts:** two items are real bugs in our parser, two were already built, and one (git hardening) needs no change. Ordered by value. "Ruleset" means whether `RULESET_VERSION` ("2026.11-0", `ruleset.rs:3`) would need a bump, which starts a new ledger chain.

---

### 1. Compaction summaries are scored as human prompts (bug, from Steer's filters)
**Evidence.** Of 650 human-classified prompts in my logs, **3 have `isCompactSummary: true`** (also `isVisibleInTranscriptOnly`) and start with "This session is being continued from a previous conversation". `line.rs:94-98` treats `origin.kind == "human"` as the whole test, so each becomes a `HumanPrompt` with a huge `chars` and, because the summary quotes project vocabulary, a high `relevance_pm`. That earns the top prompt cap (`caps.rs:40`, up to 1200 s) for text nobody typed.
**Change.** In `claude_code/line.rs::user`, before the `human` check, return `vec![]` if any of `isMeta`, `isCompactSummary`, `isVisibleInTranscriptOnly`, `isVirtual` is true. Add Steer's prefix list as a second guard on the text (`claude.rs:183`: `<task-notification>`, `<local-command-stdout>`, `<local-command-stderr>`, `<bash-stdout>`, `<bash-stderr>`, `Caveat:`, "This session is being continued…"), because the `origin`-absent fallback path (3 of 368 in FORMAT.md) has no other protection.
**Test.** Two fixture lines in `tests/fixtures/claude_code` (compact summary with `origin.kind: human`, and an `origin`-less line with an injected prefix), plus cases in `line.rs` tests next to `human_prompt_vs_task_notification_vs_sidechain`. Record the flags in `FORMAT.md` deviations.
**Ruleset.** No (parser only). Already-finalized history isn't rescanned (`finalized_through`), so past summaries stay counted unless `repair.rs` re-reads. Decide whether to backfill.
**Size.** ~15 lines + fixtures.

### 2. Human tool rejections are dropped (bug, from Steer's `toolDenialKind`)
**Evidence.** My logs have **27 `user` entries with `toolDenialKind: "user-rejected"` and no `permissionDecision`**. `line.rs:118-128` emits a `ToolDecision` only when `permissionDecision.source` exists and isn't `config`, so all 27 human denials produce a `ToolResult` and nothing else. (The other kinds seen are `automode-unavailable` ×4, `permission-rule` ×1, `interrupted` ×1. Those are automatic or already an `Interrupt`, so ignore them.) A rejection is the clearest "human decision" there is and carries `DENY_CAP_MS` = 180 s (`caps.rs:14`).
**Change.** In the same function, if no `permissionDecision` is present and `v["toolDenialKind"] == "user-rejected"` with a `tool_result` in the content, push `ToolDecision { decision: Deny, n: 0 }`. The id stays `tool_decision:0`, so a later parser that sees both forms dedups.
**Test.** Fixture line plus a `line.rs` case. Update `FORMAT.md` answer 1 (it says rejects are all `source: config`, which was true of the dirs measured then).
**Ruleset.** No. Same backfill caveat as item 1.
**Size.** ~8 lines.

### 3. Typed vs pasted (Hindsight `typed_permille`, Kairos `pasted`, Steer `typed_part`)
**Evidence.** 24 of my human prompts contain `<pasted_content id="…">…</pasted_content id="…">` blocks. **Pastes are 99% of those prompts' characters**; median total 1,569 chars vs median **20 typed**; **11 of the 24 have under 20 typed chars**. Today `features.rs:84` counts `chars` over the whole text and `prompt_features` computes relevance over it too, so a pasted file full of project identifiers inflates `relevance_pm`. `novelty_bytes` against the context tail only catches pastes of what's already in context.
Format note: Kairos's `unwrap_pastes` (`core/src/parse.rs:294`) handles the 2.1.x closing tag with the id. **Steer's `typed_part` expects `</pasted_content>` without an id and misses it**, so don't copy that one. My logs also contain a bare `</pasted_content>` and `<pasted_content ...>` variants, so match the tag family and cap id length like Kairos does.
**Change, two steps so we measure before we change hours.**
- A. In `features.rs::prompt_numbers` (and a small `split_pastes(text) -> (typed, pasted_chars)` helper next to it), emit a new numeric feature `pasted_pm` = pasted chars × 1000 / total chars. Don't change scoring. Add the key to `kind_features` (`score.rs:165`) and the README privacy list, because it's a new numeric text feature.
- B. After a few weeks of data, decide whether relevance should be computed on typed text only, with pasted text counting toward `novelty_bytes` but not `relevance_pm`. Pasting a fresh stack trace with "fix this" is legitimate and is the workflow Tally is built around, so don't zero it. That's a calibration call with labelled sessions.
**Test.** Unit tests for the splitter on all tag variants above and on an unclosed tag (must not panic or drop text).
**Ruleset.** Step A adds a feature key to hashed ledger entries, so yes, bump. Step B also.
**Size.** A ≈ 40 lines. B ≈ 10 lines plus calibration.

### 4. Features and reasons are unvalidated on the server (StruggleTime `FORBIDDEN_KEYS`)
**Evidence.** `LedgerEntry` (`ledger.rs:12`) has `kind: String`, `reasons: Vec<String>`, `features: BTreeMap<String, i64>`. `verify_chain` (`ledger.rs:96`) checks only `prev_hash` and `hash`, and `checkBatch` (`server/src/ledger.ts:55`) adds ruleset and project. A modified client can upload any feature key or reason string, and the server stores and publishes them to `tally-data`. The README promises "numeric text features" only, and today that holds only because our own daemon behaves.
**Change.** Add `validate_entry(e: &LedgerEntry) -> Result<(), &'static str>` in `tally-scoring/src/ledger.rs`: `kind` must be one of the strings from `kind_features`, every reason must be in the list at `score.rs:140-147`, and every feature key must be in that kind's allowed list. Call it from `verify_chain` (so the same Rust runs in the daemon and, via WASM, in the server) with a new `bad_shape` reason. Keep it ruleset-scoped: the allowlist lives next to `RULESET_VERSION` so a new ruleset can add keys.
**Test.** Entry with feature `"hello_world": 1` fails `bad_shape`; every claim `score()` produces passes (reuse `synthetic(4000)`).
**Ruleset.** No for 2026.11-0 *if* the allowlist matches what's emitted today. Add item 3's `pasted_pm` in the same change as its bump.
**Size.** ~40 lines.

### 5. Late-upload flag on ledger entries (Sablier `flags.go`)
**Evidence.** The server already flags pushes (`projects.ts:152`: `pushed_late` at 7 days, `claimed_after_receipt`). `ledger_entries` already has `received_at timestamptz default now()` (`001_init.sql:29`) and an index on `(chain_id, ts)` (`105_…sql`). Nothing compares an entry's own `ts` to when it arrived.
**Change.** Compute at read time, no migration: `late = received_at - to_timestamp(ts/1000)`. Buckets like Sablier: under 10 min none, under 24 h `late`, under 72 h `stale`, over that `backfill`. Return the bucket in the project/profile ledger endpoint (`server/src/projects.ts`, next to the push-receipt query) and show the count on the project page. **Advisory only**, as in Kairos `signals.rs`: it must never change `verified_s`.
**Test.** SQL-level test in `server/test` with two entries inserted with different `received_at`.
**Ruleset.** No. **Size.** ~30 lines TS.

### 6. Agent-quiet rule for commit attribution (Steer `AGENT_QUIET`, `evidence.rs:32,182`)
**Evidence.** `git.rs:39` marks a commit as the agent's only if `has_agent_commit_marker` finds a Claude Code Bash-commit marker within −120 s/+5 s (`store.rs:650`). The Copilot adapter emits `AgentBusy` (`adapters/copilot.rs`) but no markers, so a Copilot-made commit becomes `Actor::User`, `is_human_action()` is true, and it claims up to `COMMIT_CAP_MS` = 600 s. The same will happen for each adapter we add.
**Change.** In `StoreLookup::is_agent_commit`, also return true if any `agent_busy` event from any source in the same project group covers `[commit_ts - 60 s, commit_ts]`. That needs one store query next to `has_agent_commit_marker`. The failure direction is safe: a user commit made while an agent runs in the same project is under-credited, not over-credited.
**Test.** Next to the existing `agent_commit_marker_matches_by_agent_project_and_window_only` (`store.rs:668`), plus a `git.rs` test with a fake `AgentCommitLookup`.
**Ruleset.** No (it's attribution in the daemon, but it changes emitted `Commit.by`; note it in the plan). **Size.** ~25 lines.

### 7. Codex adapter: start the spike from Steer's field names (Steer `codex.rs`, `content_item_kinds`)
**Evidence.** `docs/adapters/codex.md` says "unsupported, spike not run", with every path marked UNVERIFIED. Steer reads `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl` and `response_item` entries where `internal_chat_message_metadata_passthrough.content_item_kinds` lists `user.text` / `user.image` for what the person typed and e.g. `agents_md.instructions` / `environments.environment_context` for injected context. It also reads `turn_context.approval_policy` and `event_msg task_complete` / `turn_aborted`.
**Change.** Treat all of that as hypotheses to confirm in the spike, not as the format. Put them in `codex.md`'s unverified section with Steer `codex.rs` as the source, then verify against a real session the way `FORMAT.md` did for Claude (structure-only script, counts, no values). Human prompt rule to test: item kind starts with `user.`. Reuse `claude_code/line.rs`'s split between a pure `classify(line)` and a stateful session.
**Ruleset.** No. **Size.** Doc now, adapter later. Blocked on a logged-in Codex session.

### 8. Property test: claims can't exceed wall time (StruggleTime per-drain tick cap)
**Evidence.** Intervals chain from the previous human action (`score.rs:65`), so a flood of actions can't create time; but nothing in the tests asserts it, and plan 12's red-team found the scorer caps per action. A regression here (for example an interval that restarts) would be invisible.
**Change.** In `score.rs` tests, using the existing `synthetic(n)`: assert `Σ(verified_s + unverified_s) * 1000 <= (last_human_ts - first_human_ts) + FIRST_ACTION_MS * new_runs`, counting `first_action` reasons for `new_runs`. Add a second case with 1,000 events sharing one `ts`.
**Ruleset.** No. **Size.** ~25 lines.

### 9. Replay: draw idle and agent-busy as hatched blocks (Hindsight)
**Evidence.** `replay.rs:13` already lists `agent_busy` and `idle` in `REPLAY_KINDS`. A grep for idle/busy/removed in `dashboard/src/components/Timeline.tsx` and `Replay.tsx` finds nothing, so the data is there and the UI ignores it.
**Change.** Render those two kinds as hatched, non-counting spans (CSS `repeating-linear-gradient`) in `Replay.tsx`, with a legend line "time removed from the total". Confirm first that the page payload keeps them (`replay_page` filter). Reasons per claim (`idle_subtracted`, `agent_busy_subtracted`) can link to the span.
**Test.** One case in `views.test.tsx`. **Ruleset.** No. **Size.** ~40 lines TSX/CSS.

### 10. Persona simulator for the red-team (Kairos `crates/sim`)
Plan 12's red-team is hand-written attacks. Grow `synthetic()` in `score.rs` into named personas (idler pressing a key every 59 s, prompt-spammer with trivial prompts, tab-switcher, fast-pasting user, honest user) in `tally-scoring/examples/`, each with an expected hour range, and assert it in CI. Steer's own trust page says a rhythm-faking script earned 4–6 h/day, so include that persona first (even intervals, minimal prompts) and check the per-hour yield against `TRIVIAL_PROMPT_CAP_MS` (90 s per prompt, `caps.rs:6`). **Ruleset:** none. **Size:** ~100 lines. This one finds problems rather than fixing them, so it should come before more tuning.

### 11. Optional wrapper: typed/paste/focus counts (Sablier `launcher/input.go`)
Largest item, and the only one that adds a new capture mode. Also notable: `presence::supported()` is `cfg!(windows)` (`presence.rs:37`), so today **macOS and Linux have no presence signal at all**. A `tally run -- claude` PTY wrapper that counts key chunks, bracketed-paste (`\e[200~`) and focus events (`\e[I` / `\e[O`) in 5 s windows and emits `Idle` spans would cover both gaps at once, but it only sees sessions started through it. For non-Windows presence alone, a platform `idle_ms()` is smaller and fits the existing `Platform` trait (`presence.rs:15`). Counts only, no content, as in Sablier. Defer until items 1–10 land, and decide which of the two shapes you want.

---

### Checked and dropped
- **Safe git evidence (Steer `evidence.rs`).** Audited: Tally only runs `git log --branches --no-merges --numstat`, `config user.email`, `rev-parse`, `ls-files -z` in user repos (`git.rs:122-141`, `features.rs:16`) with `GIT_OPTIONAL_LOCKS=0` (`git.rs:110`). None of those refresh the index or run fsmonitor, so the risk Steer guards against doesn't apply. No change needed. Hosting commands run in Tally's own clone under `publish/`, not the user's repo. I overstated this in the summary comment.
- **Overlap-once interval merge.** Already have it: `spans.rs` `union`/`subtract`/`clip`/`total_ms`.
- **Pure mapper tests.** Already the shape: `line.rs::classify` is pure and `Session::feed` takes entries and an output vec.
- **Approval latency (Sablier).** Already credited: an approve/deny claims the time since the previous action up to `APPROVE_CAP_MS` 60 s / `DENY_CAP_MS` 180 s. The real gap was item 2.
- **"Don't guess without a lower bound" (Sablier).** Tally's guess is `FIRST_ACTION_MS` 120 s once per run, labelled `first_action` and also bounded by the action cap, so it's small and visible. No change recommended.
- **Late flags on pushes.** Already exist (`pushed_late`); item 5 covers the remaining gap on entries.

### Suggested order
1, 2 (parser bugs, no ruleset change) → 8 (test) → 4 (+ 3A in the same ruleset bump) → 6 → 5 → 9 → 10 → 7 when a Codex session is available → 11 last.
