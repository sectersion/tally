# Tally

**Open-source AI time tracking that counts the hours of human decisions, not agent activity.**

Tally passively measures the hours of *productive struggle* you spend building projects with AI. Hackatime counts how active your editor (and your AI agent) is. Tally counts how often **you** decide something, and how long you spent getting to that decision.

> Status: **pre-alpha, in planning.** Built for the [Hack Club AI time tracking bounty](PITCH.md). Nothing to install yet.

## How it works

1. **Passive capture.** A small local daemon reads what already exists on your machine: AI harness session logs (Claude Code, Codex, Cursor, Aider, Copilot, Gemini CLI), WakaTime-compatible editor heartbeats, and git. Presence comes from the OS idle timer. No keylogging, no screenshots, no window titles.
2. **Time-to-decision.** Every human action (a prompt, an interrupt with a redirect, accepting or rejecting a change, an edit that survives, a test run, a commit) claims the human-present time since your previous action. Idle time and time spent watching the agent generate are subtracted.
3. **Substance caps.** "try again" claims about a minute; a specific, relevant prompt or a real fail→pass loop claims up to about 20. Excess goes to an *unverified* bucket instead of disappearing.
4. **Struggle check on submission.** When you submit a project, you can send an evidence bundle to a pinned, self-hosted, open-weight judge that labels each turn. Hours are re-derivable from the stored labels.
5. **Trust anchors.** A hash-chained ledger committed in real time, server-witnessed GitHub pushes, and a cap based on how the repo actually evolved.

## Privacy

- **Logged hours:** only an action ledger leaves your machine (project, action type, seconds, reason code). No text, titles, or URLs. Nothing about projects you don't track.
- **Submitted hours:** prompt text leaves only for projects you choose to submit, goes to a self-hosted judge, and is deleted after review.

## Why keep it installed

A local dashboard focused on getting better at building with AI: bugs fixed, time from failure to fix, how often your prompts used what you learned from running the code, and replays of your sessions. Public profiles show verified hours next to linked commits.

## How this badge is made

The number is verified hours only, all-time for this repository, counted from the day the daemon first ran on the maintainer's machine (`<start date: filled in at first publish>`). Earlier work, and time while the daemon was off, are not counted.

The `tally-data` branch holds `sectersion/tally/badge.json` and `sectersion/tally/ledger-<ruleset>.json`. They update about once a day, and the badge can lag up to about 15 minutes behind a publish because of caching.

To check the ledger yourself (no daemon or account needed):

```
curl -LO https://raw.githubusercontent.com/sectersion/tally/tally-data/sectersion/tally/ledger-2026.11-0.json
tally verify-published ledger-2026.11-0.json
```

Expected output: `ok: ledger-2026.11-0.json: N entries, project sectersion/tally, ruleset 2026.11-0, verified_hours N.N`.

This proves the chain is intact and the hours add up. A ledger holds claims, not events, so it cannot be re-scored. Each entry's timestamp is public and permanent in the branch history. A new ruleset starts a new file, and the badge sums all files.

## Planned stack

- **Daemon:** Rust, one binary for Windows, macOS, and Linux. Target under 50 MB RAM and under 1% CPU on a 10-year-old laptop.
- **Scoring:** one Rust crate, compiled into the daemon and to WebAssembly for the server, so client and server compute identical hours.
- **Server and dashboards:** TypeScript, Postgres, server-sent events.

## Docs

- [PITCH.md](PITCH.md): the full design, threat model, cost estimate, and evidence plan.

## License

[AGPL-3.0](LICENSE)
