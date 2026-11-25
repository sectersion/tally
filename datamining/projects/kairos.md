## Kairos (hackclub/kairos): HQ's own tracker, a direct competitor

Not a member submission. Rust workspace (core, agent, cli, server on Axum + Postgres, sim, release), ~55k lines, one squashed commit (v0.1.18), AGPL-3.0, so we can port code with attribution. The README says it "tracks your focus time while you build with AI". The landing page splits a session into focus time and excluded "agent time" (example: 35 min session, 16m40s focus, 18 min agent removed). Spot-checked `credit.rs` constants (`THINK_SECS`, `ANSWER_THINK_SECS`, `REVISION = 7`). Not read: `web/review.rs` (1,459 lines), `api/events.rs` and ingest validation, auth and trust-level logic, the agent recorder, `crates/sim`, the extension. **I did not find payout or approval logic, so whether this is "the official method" is unverified.**

**Capture.** ~13 harness readers in `crates/core/src/sources/*.rs`: Claude Code, Codex, Cursor, Copilot, Gemini, Cline, opencode, Qwen, Pi, Hermes and others. The `kairosd` daemon (`crates/agent`) writes a hash-chained event log (`core/src/event.rs`: `seal`/`digest`) with Prompt (word count, `pasted` flag), AgentStart, ReplyDone (words, tool count, files changed), Answered, Focus (window class), Present (per-second input bitmask sent once a minute), Idle, FileSave, Commit, Heartbeat.

**WakaTime interception.** It intercepts wakatime-cli heartbeats locally and forwards them to the previous endpoint, usually Hackatime (`agent/src/wakatime.rs`). It takes over the WakaTime config and the API port. **Tally must not also rewrite `~/.wakatime.cfg`: two daemons doing this will fight** (`keep`/`park` logic).

**Credit** (`core/src/credit.rs`, `credit_v8`, causal, server-side per user, old v1-v7 kept so stored credit can be rebuilt on a `REVISION` bump):
- `READ_WPM` 230, `TYPE_WPM` 35, pasted prompts count at most 60 typed words, prompts under 3 words get nothing.
- `THINK_SECS` 120, `LOOKBACK_SECS` 600. Answering an agent question: 30 s think time, capped 600 s. Replies can be read for 7200 s after finishing.
- Gaps of 180 s or less are bridged if the person was active for 5 s on both sides. While an agent runs, only 30 s after the person's last input counts.
- Editor time during an agent turn counts only within 120 s of a human write heartbeat. Heartbeats more than 600 s late are ignored.
- It is time-window based (reading, typing and answering earn bounded time), not per-action claims with substance caps like ours.

**Trust and privacy.** Same privacy contract as ours: times, counts, project names and file lists only, no code, prompts, replies, titles or URLs, no screenshots. Controls: `kairos exclude`, `pause`, `uninstall`. The chain is not witnessed by the server as far as I read. `server/src/signals.rs` has advisory signals that never change credit (days over 12 h, 4 h unbroken input, overlapping devices, uploads over 48 h late, 5 h with no output, timestamps too far off). Real trust is a human reviewer (`web/review.rs`) plus a per-user `trust_level`.

**Integration.** The read API mirrors Hackatime's (`/api/v1/users/{u}/stats`, `projects`, `heartbeats/spans`, `trust_factor`, leaderboards), with `kp_` program keys (`server/src/programs.rs`). I found no documented third-party submission path.

## Verdicts
| Item | Verdict |
|---|---|
| Session-log readers in `core/src/sources/*` for harnesses we both cover | **steal** (AGPL both ways). Biggest win in this whole issue. Diff against our adapters for formats we're missing |
| Human write heartbeat required for editor credit during an agent turn | **adapt** |
| `human_lines` / `ai_lines` heartbeat fields to filter agent-written lines | **adapt** |
| Late-heartbeat cutoff (600 s) | **adapt** |
| Advisory-only signals that never change credit | **adapt**. Keeps reviewers informed without rule creep |
| Versioned credit rules, old versions kept, rebuild on bump | **already have** (ruleset ids and one ledger chain per ruleset). Compare what happens to an already-published ledger |
| `crates/sim` persona-based simulator | **steal** the idea. Good fit for plan 12's red-team |
| Read API shaped like Hackatime's | **consider** a compatible read endpoint so existing consumers can swap the base URL |
| Time-window credit model | **avoid**. Our per-action claim plus substance caps is stricter against idling and automation |
| Rewriting WakaTime config / taking the API port | **avoid**, and make sure we coexist |
| Trusting client timestamps without a witness | **avoid**. Our server-witnessed pushes are a real differentiator |

**Positioning.** If Kairos is the HQ baseline, Tally's pitch is witnessed hashes and a per-action substance cap, not another time tracker. Worth checking in #hq whether it is the official method before more time goes into overlap.
