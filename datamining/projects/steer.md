## Steer (duck/Ellie): closest to Tally's design, v0.5.9

Rust client ~5.5k lines, Node server ~2.2k lines, AGPL-3.0-or-later (compatible with ours). Action-claim model like ours. Spot-checked `AGENT_TIMEOUT = 600` (`analyze.rs:11`), `gap_credit` (`analyze.rs:317`) and `AGENT_QUIET = 60` (`evidence.rs:32`). Not read: `live.rs` (1374 lines), `watch.rs`, `editor.rs`, `checkin.rs`, most of `server.mjs`, the browser extension, and the live site.

**Capture**
- Claude Code: hooks plus `~/.claude/projects/*/*.jsonl`, skipping `isSidechain`. Fields it reads: `isMeta`, `isCompactSummary`, `origin.kind`, `turnOrigin`, `promptSource`, `toolDenialKind == "user-rejected"`.
- Codex: `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl` (`session_meta`, `turn_context.approval_policy`, `event_msg task_complete/turn_aborted`, `content_item_kinds`).
- Copilot (`~/.copilot/session-state/<id>/events.jsonl`) and OpenCode (`opencode.db`, read-only) are marked in the source as written from docs and not tried against real use.
- Also a VS Code/Cursor extension and a Chrome extension for chatgpt.com and claude.ai. No Aider or Gemini CLI.

**Human vs agent time**
- The gap from the agent's last step to the next prompt goes through `gap_credit`. The first 60 s count in full (300 s if the human acted first or tested), then an exponential fade (240 s time constant), a ~300 s max, and a break above 1800 s credits only 120 s.
- Typing = chars/4 s clamped to 5-300 s, pasted text excluded.
- Every prompt is capped by wall-clock since the previous human action. First prompt in a session is capped at 120 s. Generic prompts are capped at 120 s.
- **Dark-flow rule:** three or more generic or near-repeat prompts within 900 s with no test keep only 30 s each.
- Interrupts, shell commands, slash commands and rejections have their own clamps (5-300 s, 10-300 s, 5-30 s, 10-60 s).
- Their idle events exist but the README says they don't count toward anything yet.

**Trust**
- Server stamps arrival time, drops events more than 300 s off, token buckets, 20k events per account per day, 12 h/day and 60 h/week caps.
- Heuristic flags (replies ~8 s after a stop, very even intervals, long days with no break), commits checked against the GitHub API within 15 min, human reviewers can approve, cut or reject.
- **`web/trust.html` admits a rhythm-faking script earned 4-6 h/day undetected.** No hash chain, no client signing.

**Privacy.** Counts only (words, chars, images, specific/observation/generic/repeat), project name, session id, timestamps. The README lists every field. Risks: self-update downloads a release binary with only a size and `steer version` check, `curl | sh` install, edits `~/.cursor/hooks.json` and Claude settings, pages public by default, reviewer API exposes per-user events. **Good:** `evidence.rs` avoids `git status` / `git log` in untrusted repos (fsmonitor and filters can run code) and uses `ls-files`, `rev-parse`, `cat-file` with timeouts.

## Verdicts
| Item | Verdict |
|---|---|
| `claude.rs` `user_event` filters (`isMeta`, `isCompactSummary`, `origin.kind`, injected prefixes) | **steal**. Check ours against it for missed injected-prompt kinds |
| `toolDenialKind == "user-rejected"` as a decision signal | **steal**. Fits "hours of decisions" |
| Codex `content_item_kinds` (typed vs injected AGENTS.md) | **steal** |
| Pasted text excluded from typing time | **steal** |
| Safe git evidence (no `git status`/`log` in untrusted repos) | **steal**. Audit our git adapter |
| Overlap-once interval merge, git "agent quiet for 60 s" rule | **steal** |
| `gap_credit` exponential fade, no cliff | **adapt**. Constants were fitted to their own sim, refit on ours |
| Dark-flow loop rule | **adapt** into our substance caps |
| Server skew correction (events more than 300 s off) | **adapt** (we have witnessed pushes, add skew) |
| Rhythm-faking red-team | **do it ourselves**. Plan 12's red-team already found the scorer caps per action, not per hour. Run their script idea against us |
| English word lists for observation/generic classification | **avoid** (gameable, English-only, duplicated in JS and Rust with a parity test) |
| Unsigned self-update, public-by-default pages, adapters listed as supported but untested | **avoid** |
