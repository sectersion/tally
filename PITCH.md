# Tally: counting human decisions, not agent activity

> Hackatime counts how active your AI agent is. Tally counts how often **you** decide something, and how long you spent getting to that decision.

This document is Tally's pitch for the [Hack Club AI time tracking bounty](#the-bounty): the design, the threat model, the cost estimate, and the evidence we will produce before asking anyone to trust the hours.

## Contents

- [The problem](#the-problem)
- [How Tally works](#how-tally-works)
- [Why Hack Club should trust the hours](#why-hack-club-should-trust-the-hours)
- [Privacy](#privacy)
- [Cost](#cost)
- [Platforms and hardware](#platforms-and-hardware)
- [Why people keep it installed](#why-people-keep-it-installed)
- [Known limits](#known-limits)
- [Evidence before we ask](#evidence-before-we-ask)
- [The path to 1,000 weighted projects](#the-path-to-1000-weighted-projects)
- [Requirements checklist](#requirements-checklist)
- [The bounty](#the-bounty)

## The problem

Hours of productive struggle toward shipped projects is still the right metric. One-shotting a project takes two minutes of struggle; reverse engineering takes dozens of hours. But in Zach's words, Hackatime "doesn't currently count how much time you are actively solving problems when building with AI. It just counts how active your AI agent is."

In Stardance, 123,400 Hackatime hours were submitted and 27,600 were approved after hand review. Many of the rest reflected people not pushing themselves, and "another big contributor was people using AI without proper self-declaration." Reviewers had to deflate hours by hand.

Two things make this hard:

1. **Projects written 100% by AI can be legitimate and skilled.** So "how much code did a human type" can't be the signal.
2. **Attention alone isn't struggle.** Someone present and prompting every few minutes can still be in dark flow, doing the same thing over and over.

## How Tally works

### 1. Passive capture

A lightweight Rust daemon reads data that already exists on your machine:

- **AI harness session logs and hooks:** Claude Code, Codex, Cursor (through its hooks), Aider, Copilot, and Gemini CLI. Each harness is a plugin of a few hundred lines that turns its log format into standard events.
- **Editor heartbeats:** the daemon accepts the WakaTime heartbeat format, so the existing WakaTime-compatible editor plugins work by pointing `api_url` at the local daemon.
- **Git:** commits, diffs, and reverts.
- **Presence:** the OS idle timer (`GetLastInputInfo` on Windows, `CGEventSourceSecondsSinceLastEventType` on macOS, XScreenSaver on X11, Mutter IdleMonitor or `ext-idle-notify` on Wayland). These need no special permissions.

Tally never logs keystrokes, takes screenshots, records window titles, or installs keyboard hooks.

### 2. Time-to-decision attribution

Every **human action** earns the human-present time since your previous action, on any project. A human action is:

- a prompt,
- an interrupt followed by a redirect,
- approving or rejecting an agent's change,
- an edit that survives (measured as surviving diff, not keystrokes),
- a test or build run,
- a commit made by you (agent commits are identified from transcript tool calls).

From each claim, Tally subtracts:

- time the agent spent generating while you were idle for more than ~45 seconds, so watching generation earns nothing, but reading and testing the output does;
- any idle stretch longer than ~3 minutes, so lunch breaks and phone time earn nothing.

Editor heartbeats caused by the agent's own file writes are discarded.

This single attention timeline handles the cases Zach named: a 5-minute session before school, hours of flow, and five projects in one afternoon. Agents running in the background earn nothing; only human actions claim time.

### 3. Substance caps

Each action's claim is capped by how much it contributed:

| Action | Can claim up to |
|---|---|
| "try again", "ok", a 1-character commit | ~1–2 min |
| A specific, relevant prompt; a real fail→pass loop; a substantial edit that survives | ~20 min |

**Relevance** is computed locally: a prompt's new information only counts if it overlaps with the project's identifiers, file names, error strings, or the agent's last output. Padded or random text scores zero. This uses a local token index, not embeddings, so it is cheap on old laptops.

**Fail→pass loops** only count when the failing test existed before the loop began, so planting `assert False` and fixing it earns nothing.

Time beyond a cap isn't deleted. It goes into an **unverified** bucket with partial linear credit, visible to reviewers, who can approve it.

All thresholds here are starting values, to be calibrated against human-labeled sessions.

### 4. Struggle check at submission

When you submit a project, you choose to send an evidence bundle for that project only:

- your prompts, verbatim;
- the agent's replies, with tool output trimmed to stubs;
- timestamps, test and build results, and linked commits;
- secrets scrubbed on your machine first.

A pinned, open-weight judge model labels each turn: steering, testing, debugging, or repetitive "try again" and dark-flow turns. Repetitive turns lose their claimed time.

- The judge never touched the original work.
- Session text is passed strictly as data, never as instructions.
- The judge's weights, prompt, and version are public.
- Its labels and rationale are stored. Hours are a deterministic function of the event timeline, the stored labels, and the rule version, so anyone can re-derive them.

The judge routes work to humans. Reviewers can overturn any label, and the least confident turns are shown first.

### 5. Trust anchors

- **Hash-chained ledger**, committed to the server in near real time, so history can't be backfilled.
- **Server-witnessed pushes:** the server records when GitHub push webhooks arrive. Commit timestamps can be forged; our receipt time can't. This anchors the clock, not the effort, and we don't claim more.
- **Repo-delta cap:** credited hours are capped by how much the repository actually evolved (iteration on the same code, fixes after failures, test growth, reverts).
- **Fraud Squad sampling,** starting with the turns the judge was least confident about.

## Why Hack Club should trust the hours

- **AI use is declared automatically** for every supported harness, per turn. That addresses a named Stardance deflation cause for honest users.
- **Agent activity earns zero.** This is the exact fix Zach asked for.
- **Reviewers audit instead of grading from scratch.** Each project shows its hours, the minutes that were removed and why, and its least certain turns. We expect review time to drop from about 20 minutes per project to about 2, and we will measure it.
- **We don't claim to prevent fraud.** We claim faking hours costs about as much as real work.

### Threat model

| Attack | Result |
|---|---|
| Lunch break, phone, YouTube while the agent runs | Idle time is subtracted |
| Several agents in parallel | Only human actions earn time |
| A scripted `claude -p` or `codex exec` loop | No presence, and templated prompts get flagged by the judge |
| Padded or random prompts | No relevance, so the claim is capped at 1–2 min |
| Planting a failing test, Esc spam, trivial commits | Each is neutralized by its own rule |
| Hourly bot pushes | Server-witnessed time without human actions earns nothing |
| Prompt injection inside the transcript | The judge only routes to humans; red-teamed with published results |
| **Hardware input faker, software input jiggler (e.g. `mouse_event`), fully modified client** | **Out of scope.** Idle is read from `GetLastInputInfo`, which any process in the session can reset; detecting that needs input hooks, which Tally promises not to use. Faking work live costs real time, and sampling remains |

## Privacy

- **Logged hours:** everything that can leave your machine is listed here. Sync uploads ledger entries (project, action type, action id, timestamps, features, seconds claimed, reason code) and head hashes with their commit timestamps. Publish writes badge data and public ledger files to the `tally-data` branch. No text, no window titles, no URLs, and nothing about projects you don't track.
- **Submitted hours:** scrubbed session text leaves only on an explicit `tally submit` (planned, plan 10; not built yet), for projects you choose to submit. It goes to a **self-hosted** judge, never a third-party model provider, and is deleted after review. Only labels and rationale are kept.
- **No keylogger APIs, no screen capture, no all-sites browser permissions.**
- A one-page explainer for parents ships with the installer.

## Cost

Scale: 15,488 monthly active users logging 109,759 hours per month. Budget: $500/month.

| Item | Monthly estimate |
|---|---|
| API, Postgres, real-time dashboard (Hackatime-scale load is about $55 effective) | ~$50–80 |
| Self-hosted 8B open-weight judge, batch jobs on spot GPUs (input-heavy, ~20–40 GPU-hours) | ~$20–80 |
| Object storage for evidence bundles (short retention) | <$5 |
| Code signing (Windows) and Apple developer account | ~$20 |
| **Total** | **~$100–185** |

The ledger is small: roughly 6.6M credited minutes a month at tens of bytes per action record, uploaded in batches every few minutes. These figures are estimates and will be replaced with measured numbers from the pilot.

## Platforms and hardware

- **Windows, macOS, and Linux**, as one Rust binary per platform. Wayland is supported through Mutter IdleMonitor or `ext-idle-notify`.
- **Target:** under 50 MB of RAM and under 1% CPU during active use, measured on a real 2016 Windows laptop. Numbers will be published.
- **One scoring implementation:** the hours logic is a single Rust crate, compiled into the daemon and to WebAssembly for the server, so client and server compute identical hours.

## Why people keep it installed

The local dashboard is about getting better at building with AI, not about scores:

- bugs you fixed, and your average time from failure to fix;
- how often your prompts drew on what you saw when running the code;
- replays of each session as a timeline of prompt, run, error, fix, commit.

Public profiles show verified hours next to linked commits. It's useful even if you never submit anything to a Hack Club program.

## Known limits

- **Web builders** (Lovable, Bolt, v0, ChatGPT in a tab): an optional extension with narrow, per-site permissions captures timing only, so these hours are mostly **unverified**.
- **Thinking away from the keyboard** isn't counted automatically. You can claim it with one click, and the claim is visible to reviewers.
- **Research before a context switch** may be attributed to the next project you act on. The unattributed remainder is shown to you and can be reassigned.
- **Unusual setups** (dev containers, SSH remotes, WSL edge cases) need the daemon running where the harness runs.
- **Harness log formats change** when tools update. The dashboard shows a health check per harness, and CI tests each adapter against fixtures from every supported version.
- **The judge's agreement with human reviewers is unproven** until the pilot measures it. Everything else is plumbing; this is the claim that needs data.

## Evidence before we ask

1. **The demo.** Replay one real teen's session side by side: Hackatime hours, Tally hours, and a human reviewer's verdict, with a timeline showing every minute removed and why. Include one case where Tally credits *more* than Hackatime (the five-project afternoon) and one where it credits less (agent running while the user is away). Run it live on a 10-year-old laptop.
2. **The pilot.** 30–50 teens over one You Ship We Ship cycle, graded blind by reviewers the Stardance way. We report reviewer-to-reviewer agreement (the ceiling we can't beat), Tally's precision and recall on useful hours, and a head-to-head against Hackatime. An interim result from 5–10 recorded sessions comes first.
3. **The red-team report.** Measured credit earned by every attack in the threat model.
4. **Resource and cost sheets** from real measurements.

## The path to 1,000 weighted projects

- Sign in with Hack Club Auth.
- Projects linked to the Unified Projects DB.
- Hackatime-format heartbeats accepted, so existing editor plugins keep working.
- A one-command install that detects AI harnesses automatically.
- One partner program that requires Tally for submissions.

## Requirements checklist

| Bounty requirement | How Tally meets it |
|---|---|
| AGPLv3 | Repository licensed AGPL-3.0; judge weights under an open license |
| Tracks productive struggle with AI | Time-to-decision, substance caps, judge at submission |
| Motivating on its own | Craft-focused local dashboard, session replay, public profiles |
| Passive | Reads existing logs, heartbeats, git, and idle time; no user effort after install |
| Central server, accounts, profiles, projects, git, real-time dashboard | TypeScript server, Hack Club Auth, GitHub App, server-sent events |
| Privacy friendly | Ledger, head hashes and badge data only for logged hours; text only for chosen submissions, self-hosted, deleted after review |
| All models and harnesses | Harness-agnostic core with small per-harness plugins |
| Windows, macOS, Linux on low-spec machines | One Rust binary per platform; measured on a 2016 laptop |
| Trustworthy | Agent time earns zero, caps, judge, trust anchors, sampling |
| Auditable | Re-derivable hours from stored timeline, labels, and rule version |
| Hosting under $500/month | Estimated ~$100–185/month |

## The bounty

Hack Club is offering $10,000 for the first open-source AI time tracking tool that meets the requirements above and is used to track 1,000 weighted projects in the Hack Club Unified Projects DB.
