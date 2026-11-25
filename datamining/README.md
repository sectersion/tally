# Datamining: other entries and primary sources

Tracking issue: #117. This directory holds everything found while reading the other entries to Hack Club's AI time tracking bounty, plus the primary sources (docs and open-source tools) that Tally depends on or will be compared against.

**How it was done.** Competing repos were shallow-cloned into scratch directories outside this repo and read, never run or installed. Research was split across read-only sub-agents; constants, licences and the log-format claims were then spot-checked against the source or against local logs. Measurements over Claude Code logs use structure-only scripts (`scripts/`) that print counts and lengths, never text. Each file has a "Not verified" section: trust those over the summaries.

**What this is not.** The bounty thread itself was not read (no Slack access): see `research/bounty-thread.md`. Hindsight's repo is private (404), so only its site was read.

## Headline findings

1. **Two bugs in Tally's Claude Code parser**, found by measuring real logs (`implementation/steal-writeups.md`, items 1 and 2):
   - Compaction summaries (`isCompactSummary`) are scored as human prompts. 3 of 650 human-classified prompts in one set of logs.
   - Human tool rejections (`toolDenialKind: "user-rejected"`, no `permissionDecision`) are dropped. 27 in the same logs.
2. **Kairos is HQ's own tracker and a direct competitor** (`projects/kairos.md`). Its ~13 harness parsers are the most useful code to port (`implementation/code-to-steal.md`). Whether it is "the official method" is unverified.
3. **Hooks cannot replace log parsing** (`research/claude-code-hooks-otel.md`). Hooks cannot tell typed from pasted prompts or detect a human rejection. Keep logs as the primary source; hooks are an optional second source at most.
4. **Heartbeat protocol notes — mostly already fixed in PR #109** (`research/wakatime-cli.md`): the basics (at most 10 heartbeats per bulk request, Basic auth only, no `/v1`, UUID-shaped key, bbolt offline queue, machine name as a header, the 120 s send throttle) were already verified against real wakatime-cli v2.28.4 and are documented in `docs/adapters/heartbeat.md` and the fixtures' `NOTES.md`. Still open from the source read: hidden-file-names mode nulls `lineno`/`cursorpos`/`lines` and sends `HIDDEN<.ext>` as the entity, `line_additions`/`line_deletions` are not fields in the CLI at all (ignored on stdin, not just unseen), a per-heartbeat inner 400 is dropped rather than re-queued, the throttle is global (bursts of up to 10 with original `time` values, never a live stream), and `ai_subscription_plan` is a field Tally has not seen on the wire.
5. **Reviewers will compare against Hackatime's number** (`research/hackatime.md`): a sum of gaps between heartbeats, each capped at 120 s. Its trust factor is a manual admin label, not a computed score. Recommendation: show our hours next to that figure, do not send heartbeats to Hackatime.
6. **Presence is Windows-only today**, so macOS and Linux have no presence signal (`research/activitywatch-afk.md`). ActivityWatch's watcher is MPL-2.0 and does not do Wayland itself; a fresh Rust implementation per OS is recommended.
7. **ccusage's dedupe keys on message id + request id, never `uuid`** (`research/ccusage.md`), which supports dropping the session from our event id (see the open question in `claude_code/session.rs`). Not fully settled: it is inferred, not confirmed.
8. **A commit-gap baseline for this repo** is about 20 h with git-hours' default 120-minute settings, 17 h at 30 minutes, across 164 commits in about 27 hours of wall clock (`research/git-hours-baseline.md`).

## Suggested order

1. Fix the two parser bugs (no ruleset change).
2. Add the property test that claims cannot exceed wall time.
3. Server-side ledger entry validation, together with the `pasted_pm` feature and its ruleset bump.
4. Commit attribution for non-Claude agents (agent-quiet rule).
5. Port any remaining genuinely-new heartbeat findings from `research/wakatime-cli.md` into the docs (hidden-file `HIDDEN` entities, missing `line_additions`/`line_deletions`, inner-400 handling, burst semantics, `ai_subscription_plan`); the protocol basics are already done (PR #109).
6. Late-upload flag on ledger entries, replay hatching, persona simulator.
7. Codex adapter spike, starting from Kairos and Steer's field names.
8. Non-Windows presence.
9. Read the bounty thread (someone with Slack access) before deciding positioning against Kairos.

## Index

| Path | What |
|---|---|
| `projects/sablier.md`, `steer.md`, `struggletime.md`, `kairos.md`, `hindsight.md`, `no-public-code.md` | One file per competing entry: capture, scoring, trust, privacy, verdicts |
| `implementation/steal-writeups.md` | Implementation notes for every "steal", with code references and tests |
| `implementation/code-to-steal.md` | Which code to port, from where, with caveats |
| `research/claude-code-hooks-otel.md` | Official hooks and OpenTelemetry vs log parsing |
| `research/activitywatch-afk.md` | Cross-platform idle detection |
| `research/hackatime.md` | How reviewers' existing tool computes hours and trust |
| `research/wakatime-cli.md` | Real heartbeat protocol vs our synthetic notes |
| `research/ccusage.md` | Log-format drift and dedupe rules |
| `research/git-hours-baseline.md` | Commit-gap estimators and a baseline |
| `research/bounty-thread.md` | Not done: what to read and why |
| `scripts/` | Structure-only log measurements and the git-hours baseline script |

## Licences

Sablier, Steer, StruggleTime and Kairos are AGPL-3.0 (checked in each `LICENSE`), the same as Tally. Hackatime, git-hours and ccusage are MIT, wakatime-cli is BSD-3-Clause, aw-watcher-afk is MPL-2.0 (see each report for what that implies).
