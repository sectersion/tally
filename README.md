# Tally

[![tracked with Tally](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Fsectersion%2Ftally%2Ftally-data%2Fsectersion%2Ftally%2Fbadge.json)](https://github.com/sectersion/tally/tree/tally-data/sectersion/tally)

**Open-source AI time tracking that counts the hours of human decisions, not agent activity.**

Tally passively measures the hours of *productive struggle* you spend building projects with AI. Hackatime counts how active your editor (and your AI agent) is. Tally counts how often **you** decide something, and how long you spent getting to that decision.

> Status: **pre-alpha.** Built for the [Hack Club AI time tracking bounty](PITCH.md). The daemon, the [server and dashboards](https://tally.dino.icu) are live. No published release yet, so the one-line installers have nothing to download: build from source (see below).

## How it works

1. **Passive capture.** A small local daemon reads what already exists on your machine: AI harness session logs (Claude Code and Copilot today; Codex, Cursor, Aider and Gemini CLI are detected but have no adapter yet), WakaTime-compatible editor heartbeats, and git. Presence comes from the OS idle timer. No keylogging, no screenshots, no window titles.
2. **Time-to-decision.** Every human action (a prompt, an interrupt with a redirect, accepting or rejecting a change, an edit that survives, a test run, a commit) claims the human-present time since your previous action. Idle time and time spent watching the agent generate are subtracted.
3. **Substance caps.** "try again" claims about a minute; a specific, relevant prompt or a real fail→pass loop claims up to about 20. Excess goes to an *unverified* bucket instead of disappearing.
4. **Struggle check on submission.** When you submit a project, you can send an evidence bundle to a pinned, self-hosted, open-weight judge that labels each turn. Hours are re-derivable from the stored labels.
5. **Trust anchors.** A hash-chained ledger committed in real time, server-witnessed GitHub pushes, and a cap based on how the repo actually evolved.

## Install and use

What works today, from a clone of this repo:

- `curl -fsSL https://raw.githubusercontent.com/sectersion/tally/main/install/install.sh | sh` (macOS/Linux) or `irm https://raw.githubusercontent.com/sectersion/tally/main/install/install.ps1 | iex` (Windows) installs the binary — but there is no release yet, so it falls back to the sources it can find. From a clone, `cargo install --path crates/tally-daemon` puts `tally` on PATH.
- The Claude Code plugin installs from the clone: `claude plugin marketplace add <path to clone>`, then `claude plugin install tally@tally`.
- The VS Code extension is built from `integrations/vscode` with `bun install && bun run package`, which writes `tally-vscode.vsix`; `tally setup --only vscode --extras-dir integrations/vscode` installs it.
- `tally setup --dry-run` shows what setup would install without changing anything.
- `tally status --brief` prints one line; `tally stop` pauses tracking across sessions until `tally start`.
- `tally service install|start|stop|status|uninstall` runs the daemon at boot (Task Scheduler / launchd / systemd).
- `tally login` links your device to the server via the device-code flow.
- `tally submit <project>` builds and uploads a scrubbed evidence bundle to a self-hosted judge. See [Privacy](#privacy).

One-line installers with real release downloads, a published plugin marketplace and VS Code Marketplace listings are planned, not built. Until then the daemon starts with your first Claude Code session or VS Code window. Details and what is verified: [docs/extensions.md](docs/extensions.md).

## Privacy

- **Logged hours:** what can leave your machine is listed here, and nothing else. Sync uploads ledger entries (project, action type, action id, timestamps, seconds, reason code, numeric text features) and head hashes with commit timestamps. Publish writes badge data and public ledger files to the `tally-data` branch. No text, titles, or URLs. Nothing about projects you don't track.
- **Submitted hours:** scrubbed session text leaves only on an explicit `tally submit`, for projects you choose, and goes to a self-hosted judge that deletes it after review. You preview and approve the bundle before it uploads.

## Why keep it installed

A local dashboard focused on getting better at building with AI: bugs fixed, time from failure to fix, how often your prompts used what you learned from running the code, and replays of your sessions. Public profiles show verified hours next to linked commits.

## How this badge is made

The number is verified hours only, all-time for this repository, counted from the day the daemon first ran on the maintainer's machine (2026-11-23). Earlier work, and time while the daemon was off, are not counted.

The `tally-data` branch holds `sectersion/tally/badge.json` and `sectersion/tally/ledger-<ruleset>.json`. They update about once a day, and the badge can lag up to about 15 minutes behind a publish because of caching.

To check the ledger yourself (no daemon or account needed):

```
curl -LO https://raw.githubusercontent.com/sectersion/tally/tally-data/sectersion/tally/ledger-2026.11-0.json
tally verify-published ledger-2026.11-0.json
```

Expected output: `ok: ledger-2026.11-0.json: N entries, project sectersion/tally, ruleset 2026.11-0, verified_hours N.N`.

This proves the chain is intact and the hours add up. A chain cut to its first N entries still verifies, so compare the entry count and hours with the badge. A ledger holds claims, not events, so it cannot be re-scored. Each entry's timestamp is public and permanent in the branch history. A new ruleset starts a new file, and the badge sums all files.

## Planned stack

- **Daemon:** Rust, one binary for Windows, macOS, and Linux. Target under 50 MB RAM and under 1% CPU on a 10-year-old laptop.
- **Scoring:** one Rust crate, compiled into the daemon and to WebAssembly for the server, so client and server compute identical hours.
- **Server and dashboards:** TypeScript, Postgres, server-sent events.

## Docs

- [PITCH.md](PITCH.md): the full design, threat model, cost estimate, and evidence plan.
- [docs/extensions.md](docs/extensions.md): the Claude Code plugin, the VS Code extension, installers and `tally setup`.
- [server/README.md](server/README.md): the ledger server (Bun + Postgres), how to run and deploy it.
- [docs/release.md](docs/release.md): how release archives are built and signed.
- [docs/cost.md](docs/cost.md): measured resource usage and server costs.

## License

[AGPL-3.0](LICENSE)

## Attribution

Everything in the datamining/ folder is external analyses and does not contain code or implementations.
Credit to flaticon.com for the "tally" favicon on tally.dino.icu.
