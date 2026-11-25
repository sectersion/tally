# Claude Code hooks and OpenTelemetry as capture sources for Tally

Researched 2026-11-24 from code.claude.com/docs (pages fetched through a summarising tool, so field lists are second-hand; see "Not verified").

## Summary

- Official hooks and OTel cover most of what Tally reads from the transcript, except the typed/queued/suggestion-accepted split. That split exists only in the log (`promptSource`).
- OTel is the only documented route to human-vs-automatic permission decisions (`tool_decision.source`). Hook inputs carry no such field.
- The hook and OTel docs publish event and field names. The transcript .jsonl has no documented schema, and one hook doc line warns it may be incomplete.
- Hooks are an optional second source for liveness and gap-filling. They should not replace log parsing.

## Findings

1. **Hook events** (https://code.claude.com/docs/en/hooks). The page lists 33 events. Relevant ones:
   - `SessionStart` (`source`: startup/resume/clear/compact/fork; `model`)
   - `UserPromptSubmit` (`prompt`)
   - `PreToolUse`
   - `PermissionRequest`
   - `PermissionDenied`
   - `PostToolUse` (`tool_name`, `tool_input`, `tool_response`, `tool_use_id`, `duration_ms`)
   - `PostToolUseFailure` (adds `error`, `is_interrupt`)
   - `Notification` (`message`, `title`, `notification_type`)
   - `SubagentStart` and `SubagentStop` (`agent_id`, `agent_type`, `agent_transcript_path`, `last_assistant_message`, `stop_hook_active`)
   - `Stop` (`last_assistant_message`, `stop_hook_active`, `background_tasks`, `session_crons`)
   - `PreCompact` and `PostCompact` (matcher `manual`/`auto`)
   - `SessionEnd` (a `reason` field appears in an example)
   
   Common fields on every event: `session_id`, `prompt_id`, `transcript_path`, `cwd`, `permission_mode`, `effort`, `hook_event_name`. `agent_id` and `agent_type` are added inside subagents.
   - `Notification` types: `permission_prompt` fires after about 6 s of waiting. `idle_prompt` fires about 60 s after Claude finishes. These two are a rough official signal of "waiting on the human".
   - `PermissionRequest` input: `tool_name`, `tool_input`, `permission_suggestions`. It has no `tool_use_id`, so it cannot be joined to a tool result by id.

2. **Distinguishing prompt kinds and decisions.**
   - Prompt kinds, hooks: not possible. `UserPromptSubmit` has no source, paste or queued field. The same hook also fires for `/loop`, scheduled tasks, background subagent reports and cross-session messages, and the input does not mark them. A paste is visible only as `<pasted_content id=...>` wrapper text in some sessions.
   - Prompt kinds, logs: Tally's FORMAT.md records `promptSource` as `typed`, `queued`, `suggestion_accepted`, `system` or `sdk`. That is log-only and not in any doc I read.
   - Human vs automatic permission, OTel: `claude_code.tool_decision` has `decision` (accept/reject) and `source`. Values are `config` (rules, flags, mode or auto classifier, with the matching source not named), `hook`, `user_permanent`, `user_temporary`, `user_abort`, `user_reject`. It also has `classifier_verdict` for auto mode. https://code.claude.com/docs/en/monitoring-usage
   - Human vs automatic permission, hooks: `PermissionRequest` input does not say who decides. `PermissionDenied` fires only for auto-mode classifier denials, not for user denials.
   - User rejection, hooks: not detectable. `PostToolUseFailure` does not fire for permission denials, and `PostToolUse` has no rejection field. `is_interrupt` marks aborts of running tools only.
   - User rejection, OTel: `source=user_reject` or `user_abort`. The code-edit metric `claude_code.code_edit_tool.decision` also carries accept/reject for Edit, Write and NotebookEdit.

3. **Prompt text and local-only hooks.**
   - Command hooks get `prompt` on stdin in plain text. I did not find any setting that stops a hook receiving it.
   - Hooks are filtered by matcher, and the `if` field narrows by tool pattern.
   - `UserPromptSubmit` takes no matcher, so a hook sees every prompt.
   - A `command` hook runs locally. `http`, `mcp_tool`, `prompt` and `agent` hooks can send data off-machine. Admin controls: `allowedHttpHookUrls`, `httpHookAllowedEnvVars`, `allowManagedHooksOnly`, `disableAllHooks`.
   - A command hook that writes to a local socket or file never leaves the machine. That is a design choice, and the docs do not call it a guarantee.

4. **OpenTelemetry** (https://code.claude.com/docs/en/monitoring-usage). Enabled by `CLAUDE_CODE_ENABLE_TELEMETRY=1`.
   - Metrics: `claude_code.session.count`, `lines_of_code.count`, `pull_request.count`, `commit.count`, `cost.usage`, `token.usage`, `code_edit_tool.decision`, `active_time.total`.
   - `active_time.total` has `type=user` (keyboard interaction) or `cli` (tool execution and AI responses). It excludes idle time.
   - Events: `user_prompt`, `assistant_response`, `tool_result`, `tool_decision`, `api_request`, `api_error`, `api_refusal`, plus body events.
   - `user_prompt` attributes: `prompt_length`, `prompt`/`prompt_text`, `message.uuid`, `command_name`, `command_source`.
   - `tool_result` attributes: `duration_ms`, `success`, `decision_source`, `tool_use_id`.
   - Prompt text is opt-in via `OTEL_LOG_USER_PROMPTS=1`; the default is `<REDACTED>` plus length. Tool parameters need `OTEL_LOG_TOOL_DETAILS=1`.
   - Human-time signals: only `active_time.total{type=user}`, an aggregate with no definition of how keyboard activity is measured. There is no permission-latency metric. The `claude_code.tool.blocked_on_user` span (beta) gives the wait duration with `decision` and `source`. There is no paste signal.
   - OTLP export is fire-and-forget. A repo's `.claude/settings.json` cannot enable telemetry or set exporters. Only user, managed or `--settings` scope can.

5. **Stability.**
   - Hooks and OTel have per-field docs and version tags (for example "v2.1.251+"). The OTel page says tracing is beta, and that content attributes under detailed beta tracing "aren't part of the stable span schema". I found no general semver promise for either.
   - The hooks page documents `transcript_path` but I found no schema for the file. It says the transcript "isn't guaranteed to include the final message at Stop time on all versions", and `MessageDisplay` ids cannot be correlated with transcript ids. The OTel page calls transcript join keys version-specific and "can break on any release".
   - A background web search returned only memory-based claims (UNVERIFIED) that the format is internal and has broken third-party parsers before.

6. **Settings interaction** (https://code.claude.com/docs/en/settings).
   - Precedence, highest first: managed, `--settings`, project local, shared project, user.
   - List keys merge across scopes, so hooks arrays from several scopes are combined rather than replaced (the page states this for lists such as `permissions.allow`; I did not confirm it for `hooks` specifically).
   - `--settings` sits above all files except managed. Injecting hooks there leaves the user's files untouched but runs in addition to any existing hooks. The Sablier approach therefore does not replace the user's hooks.
   - Risks:
     - Managed `allowManagedHooksOnly` or `disableAllHooks` silently drops our hooks.
     - Claude Code reloads settings mid-session, and a broken settings file is skipped, which can disable hooks.
     - Hook latency can block a turn. Defaults are 600 s for command hooks, so use `async` and a short `timeout`.
     - A nonzero exit is non-blocking by default, but exit 2 on `UserPromptSubmit`, `PreToolUse` and `Stop` blocks the user's work, so the hook must always exit 0.
     - Editing the user's `~/.claude/settings.json` is invasive and may be read-only or generated by another tool.
     - Project-scope hook files are shared with teammates.

## Comparison with Tally's parser

| Signal | Hooks / OTel | Logs |
|---|---|---|
| Prompt timestamp and text | `UserPromptSubmit` is live and exact. OTel has `prompt_length` by default. | Yes, from the log. |
| Typed vs queued vs suggestion-accepted | Not available | `promptSource` (log only) |
| Human vs `task-notification`/SDK turn | Not marked in the hook input | `origin.kind`, `turnOrigin` |
| Permission human vs config | OTel `tool_decision.source` | `permissionDecision.source` |
| Permission wait time | `Notification permission_prompt` (6 s threshold), or the `blocked_on_user` span | Log timestamps |
| Tool rejection | OTel `user_reject` | `[Request interrupted by user` text |
| Subagent sidechains | `SubagentStart` / `SubagentStop`, with transcript path | `isSidechain` files |
| Turn end | `Stop` (live) | `turn_duration` system entry |
| History / backfill | None | The only source |

## Recommendation

Keep log parsing as the primary source. It carries the only complete prompt-kind data and the only backfill, and Tally already depends on it. Add hooks as an optional, opt-in second source with local-only `command` hooks that exit 0 and run async:
- `UserPromptSubmit`, `Stop` and `Notification` to give timely turn boundaries and idle/permission-wait signals.
- Never send prompt text anywhere and never use `http` hooks.

OTel needs a collector endpoint, which a local daemon would have to host. It is worth it only for `tool_decision.source` and `active_time`, and only if log-derived `permissionDecision.source` proves unreliable. Treat hook data as corroboration, never as ground truth, because hooks can be disabled by policy.

## Not verified

- `SessionEnd`, `PreCompact` and `PostCompact` input field lists, and the `reason` values. Only the event names and example fragments were seen.
- Whether `hooks` arrays merge across scopes. This is inferred from the general list-merge rule.
- The OTel page was read to about 100k of 184k characters. The descriptions of the later events and the exact meaning of `user_abort` are inferred.
- Whether `active_time` `type=user` counts paste or only keystrokes.
- Any official statement on the transcript schema. None found, but I did not search the whole site. The `docs.claude.com` host was not fetched.
- The statusline and permissions pages were not read.
- Whether `Notification` fires in headless mode.
