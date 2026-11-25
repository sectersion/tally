# Red-team report

Ruleset `2026.11-1`, `tally-scoring` at commit `7c6bcd7`.

The harness drives the scorer with the events each adapter would emit. It does not run real harnesses, so the assumption column says what the adapter or judge must have done for these events to be the right input.

Honest control: 61 min credited out of 60 min.

| attack | relies on | simulated minutes | credited minutes | credited as % of honest control | assumption |
|---|---|---|---|---|---|
| `scripted_claude_p_loop` | adapter | 60 | 0 | 0% | the Claude Code adapter emits no human prompt for a `claude -p` run because the log entry's origin is not human |
| `scripted_loop_misattributed_as_human` | judge | 60 | 60 | 99% | templated prompts are detected by the judge; this row reports what the scorer alone credits |
| `padded_random_prompts` | scorer | 60 | 45 | 73% | relevance and novelty are computed by the daemon from the prompt text against the session context; random text scores relevance 0 |
| `planted_failing_test` | scorer | 60 | 60 | 98% | the adapter reports the user's own test runs as initiator user; the 'failing test existed before the loop' check is deferred (spec section 8) |
| `esc_spam` | scorer | 60 | 60 | 99% | the adapter emits one Interrupt per Esc press |
| `trivial_commits` | scorer | 60 | 40 | 65% | the git source reports each commit by the user with its changed-line count |
| `parallel_agents` | scorer | 60 | 0 | 0% | agent activity never produces a human event |
| `idle_youtube_while_agent_runs` | scorer | 60 | 6 | 11% | the presence source reports the away time as an Idle span; relevance 400 is below the think threshold so no think allowance applies |

## Not modelled here

- `hourly_bot_push`: server-side: server-witnessed time without human actions earns nothing (plan 09)
- `prompt_injection_in_transcript`: judge-side: the judge only routes to humans (plan 10)

## Per attack

### `scripted_claude_p_loop`

Pitch row: A scripted `claude -p` or `codex exec` loop.

Credited 0 s verified, 0 s unverified. Reasons: none.

### `scripted_loop_misattributed_as_human`

Pitch row: A scripted loop whose prompts are attributed to the human.

Credited 3630 s verified, 30 s unverified. Reasons: capped, first_action.

### `padded_random_prompts`

Pitch row: Padded or random prompts.

Credited 2700 s verified, 900 s unverified. Reasons: capped, first_action.

### `planted_failing_test`

Pitch row: Planting a failing test.

Credited 3600 s verified, 0 s unverified. Reasons: fail_pass_loop, first_action.

### `esc_spam`

Pitch row: Esc spam.

Credited 3625 s verified, 90 s unverified. Reasons: capped, first_action.

### `trivial_commits`

Pitch row: Trivial commits.

Credited 2400 s verified, 1230 s unverified. Reasons: capped, first_action.

### `parallel_agents`

Pitch row: Several agents in parallel.

Credited 0 s verified, 0 s unverified. Reasons: none.

### `idle_youtube_while_agent_runs`

Pitch row: Lunch break, phone, YouTube while the agent runs.

Credited 405 s verified, 0 s unverified. Reasons: agent_busy_subtracted, first_action, idle_subtracted.

## Gaps

- `scripted_loop_misattributed_as_human`: credited 60 of 60 minutes (100%), relies on the judge. 
- `padded_random_prompts`: credited 45 of 60 minutes (75%), relies on the scorer. 
- `planted_failing_test`: credited 60 of 60 minutes (100%), relies on the scorer. KNOWN GAP: spec section 8 defers the 'failing test existed before the loop' check, so this earns credit today.
- `esc_spam`: credited 60 of 60 minutes (100%), relies on the scorer. 
- `trivial_commits`: credited 40 of 60 minutes (66%), relies on the scorer. 
- `idle_youtube_while_agent_runs`: credited 6 of 60 minutes (11%), relies on the scorer. 
