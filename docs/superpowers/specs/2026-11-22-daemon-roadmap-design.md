# Tally daemon: roadmap and MVP design

Status: draft for approval. Ruleset `2026.11-0`. Scoring crate (`tally-scoring`) is merged (PRs #1, #2).

## Intent and why

The daemon turns what already exists on the user's machine (AI harness logs, git, the OS idle timer) into the events `tally-scoring` consumes, scores them, and keeps a hash-chained ledger of the result.

Why now, in the user's words: the best way to show Tally works is to use it on itself. The first goal is for the maintainer to track the time they spend building this repo with Claude Code, as a proof of concept for Hack Club HQ. When the MVP is done, the repo README shows a **badge that updates daily** with the time spent on the project.

## Goals

- First working target (sections 1 to 3): a real Claude Code session on Windows is captured, scored, and written to a local hash-chained ledger, checkable with `tally verify`.
- MVP (adds section 4, built only after the first target works end to end): the same data shown as a daily-updating badge on this repo's README, with the ledger published next to it so the chain can be checked by anyone.
- The same pipeline later serves other harnesses, other operating systems, and the real server without rework.
- Hours shown are `verified_s` only (spec §3); unverified time is never credited automatically.

## Non-goals (for the MVP)

- The TypeScript/Postgres server, Hack Club Auth, GitHub App, judge model, local dashboard.
- Harnesses other than Claude Code; macOS and Linux presence; installer and auto-detection.
- The WakaTime-compatible heartbeat endpoint.
- Tracking projects other than this repo.

## Anti-goals

- Anything that publishes prompt text, file contents, commands, window titles or URLs. Only hashes and derived numbers leave the machine (spec §2).
- A badge that can be moved by agent activity. Only human actions earn time.
- A badge number nobody can check.

## Constraints

- One Rust binary, no runtime dependencies for the user. Targets (later): under 50 MB RAM, under 1% CPU.
- The daemon uses `tally-scoring` natively. No second scoring implementation.
- Claims enter the ledger only once final (see "Finalization"). The chain is append-only.
- Windows first (the maintainer's machine).

## Sections, in order

Each section gets its own design check, plan, and implementation, one at a time.

1. **Core pipeline.** Local event store, project identity (git root becomes local id), the loop that runs `score()`, finalization of claims, local hash chain, `tally status` and `tally verify` commands.
2. **Claude Code adapter and prompt features.** Parse Claude Code session logs into events (`prompt`, `interrupt`, `tool_decision`, `agent_output`, `agent_busy`); compute `relevance_pm` and `novelty_bytes` locally with the `text` feature.
3. **Presence and git (Windows).** Idle spans from the OS idle timer, commits, test runs.
4. **Publish (MVP ends here).** Daily job that publishes the badge endpoint JSON and the ledger. See "Badge and publishing". Not needed until sections 1 to 3 work end to end (maintainer's call).
5. Other harness adapters and the WakaTime heartbeat endpoint.
6. Sync with the real server (batched upload, periodic hash commit, local-to-server project ids). The server then becomes the source of truth for the badge.
7. Local dashboard.
8. Packaging: installer, harness auto-detection, macOS and Linux idle detection, resource measurements.

## Approaches considered for the badge

- **A. Badge-endpoint JSON only, no server.** Fastest. The badge is a number the daemon asserts; nothing to check.
- **B. Wait for the real server.** Most faithful to the pitch (server recomputes hours), but the MVP waits for the entire server plan.
- **C. A, plus the ledger published beside it.** Chosen by the maintainer. No server needed, and anyone can verify the chain and sum the hours. Costs one more published file and a public project name.

## Badge and publishing (decided: approach C)

- Daily, the daemon writes a shields.io endpoint JSON (`schemaVersion`, `label`, `message`, `color`) and the ledger for this project to a public, free location the daemon can update, and the README embeds the badge through `img.shields.io/endpoint`.
- Badge shows all-time verified hours for this repo.
- Published ledger entries are the spec §6 shape with `project` set to the public repo name (`sectersion/tally`), not the local hash id. Local-to-public mapping happens before `chain()` (scoring plan Task 5 note).

### Decided by the maintainer

- **Published timestamps are fine**, permanently. Each ledger entry's `ts` shows when they work on this repo, and a git-hosted copy keeps it in history.
- **The badge and publishing come after everything else works** (section 4 is last in the MVP).

### My calls (not yet agreed; please check)

- **Live capture only.** The badge counts time from the day the daemon first runs; earlier work (including building the scoring crate) is not backfilled, and time while the daemon is off is dropped, not scored. Reason: the scorer treats missing idle data as "present", so replayed history or off-time would be over-credited. Counting history would need a new rule that treats time without presence data as unverified, which is a ruleset change and its own section. The README says when tracking began.
- **"This repo"** is the remote `sectersion/tally`. Every worktree and clone of it rolls up into one project; logs are matched to it through the git common dir. The local id stays the SHA-256 of the canonical root path per spec §1, with the mapping to the public name made before `chain()`.
- **Process model:** for the MVP the daemon is a manually started background process. No installer, no service registration.
- **Ledger changes:** append-only. A ruleset bump starts a new chain; the old chain stays published and the badge sums verified hours across chains. Corrections are appended, never rewritten.
- **The ledger proves integrity, not derivation.** A ledger holds claims, not events, so anyone can verify the chain and add up hours, but cannot re-run `score()` from it. Re-deriving hours needs the events (hashes and numbers only, per spec §2). For the MVP I publish the ledger only; publishing events too is a later, separate decision.
- **Daily means once per calendar day when the daemon is running**, and catching up at next start if a day was missed. Not a promise of a fixed time.
- **Badge label and format:** "tracked with Tally" / "`N.N h`", verified only.
- **Hosting location** is left to the builder within these constraints: public, free, no server, updatable by the daemon using a credential the user controls and can revoke.

## Finalization

`score()` is a pure function of the full event history, but a claim can still change while later events may arrive: an interrupt's cap depends on a prompt in the next 120 s, and idle and busy spans are only known once they end. The daemon therefore appends a claim to the ledger only when no later event can change it. The wait period, and how an open (still running) span is treated, are left to the builder; the wait must be at least `INTERRUPT_FOLLOW_MS`.

## Fixtures

This repo's own Claude Code session logs (`~/.claude/projects/<project>/*.jsonl`) are the adapter's first test data, with text scrubbed before commit. Observed in this machine's logs (structure only): human prompts carry `origin.kind = "human"` and `turnOrigin = "human"`; background task notifications and tool results are marked differently and are not human actions; tool approvals appear as `permissionDecision`; run time appears as `turn_duration`.

## Left to the builder

Storage format and library, concurrency model, directory layout and crate split, log-watching mechanism, finalization wait length, hosting location, CLI shape.
