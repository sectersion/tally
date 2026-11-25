# Hackatime: how it judges hours, and what Tally should do

Source: shallow clone of hackclub/hackatime at <scratch>/hackatime/src (read only, nothing run). Paths below are relative to it.

## Summary

Hackatime is a Rails app with a WakaTime-style duration model. It sums the gaps between consecutive heartbeats, and each gap is capped at 2 minutes. Its "trust factor" is a manual, admin-set label with four values. It is not a score computed from heartbeats. Its only automated checks are a timestamp sanity check, duplicate hashing and a per-IP throttle. The repo contains no automated detection of fake activity. It has first-class "ai coding" heartbeats and AI fields. The license is MIT, so borrowing logic into AGPL-3.0 is fine.

## Findings

1. **Duration math.** `app/models/concerns/heartbeatable.rb`.
   - Timeout: `heartbeat_timeout_duration` defaults to `2.minutes` (l.44-45), i.e. 120 s.
   - Per-gap formula (`with_capped_duration`, l.331-337): the gap is `LEAST(time - LAG(time), 120)`. The first heartbeat in a scope counts 0.
   - Ordering and attribution: `LAG(time) OVER (ORDER BY time, id)`, so ties break by id. Each gap is credited to the heartbeat that ends it.
   - Overlaps: no merge. Heartbeats from different files, projects or machines all sit on one timeline for the user. Two heartbeats at the same second give a gap of 0, so concurrent activity is never double-counted.
   - Grouping: `duration_seconds` (l.252-282) takes at most one group column (project, language and so on) and uses `LAG ... PARTITION BY group`. Within a group the gap is therefore measured to the previous heartbeat of that group, so per-project totals can exceed the user total.
   - Other call sites: `durations_by_day` (l.52-70) and `duration_seconds_boundary_aware` (l.286-310).
   - The API's `unique_heartbeat_seconds` (`app/controllers/api/v1/stats_controller.rb` l.205-215) only counts gaps of 1-120 s and drops longer ones.
   - Is it WakaTime's algorithm? It is the same idea (WakaTime's default timeout is 15 min). Hackatime uses 2 min and stricter capping, and I did not compare it to the WakaTime source (UNVERIFIED). Heartbeats with `time` outside 0..253402300799 are excluded (l.35-36).
   - Scoping: `coding_only` keeps `category = 'coding'`. `leaderboard_eligible` keeps coding and "ai coding", drops browser editors and `<<LAST_PROJECT>>`.

2. **Trust factor.**
   - Enum `User.trust_level` (`app/models/user.rb` l.48-53): blue 0 (unscored), red 1 (convicted), green 2 (trusted), yellow 3 (suspected). Yellow is masked to blue for everyone except verified confidential OAuth apps (`mask_trust_level`, l.55-59).
   - Inputs: only an admin action. `can_change_trust_of?` (l.137-146) requires admin+ and a strictly higher rank than the target, and setting red needs `can_convict_users?`. Changes are written to `trust_level_audit_logs`.
   - Exposure: `GET /api/v1/users/:username/trust_factor` (routes l.259; `stats_controller.rb` l.125-138), which looks users up by Slack uid, username or id. It also appears as `trust_factor` in `/stats` responses (l.103-111) and in `/api/v1/authenticated/me` (`me_controller.rb`).
   - Red effects: `api_access_restricted?` (user.rb l.319), the leaderboard `verified` flag (green) and `red` flag (`leaderboard_page_cache.rb` l.46), and deletion rules.
   - Poison: an admin API (`poisons_controller.rb`) creates a `HeartbeatExclusion` with a cutoff date. Heartbeats before the cutoff are hidden from every query by a default scope (`heartbeat.rb` l.8-9).
   - Ban triggers: the audit logs are written by admin actions only. I found no automatic conviction rule. A full-repo search for one was not done (UNVERIFIED).
   - Anti-abuse in the code:
     - `HeartbeatIngest` (`app/services/heartbeat_ingest.rb`) accepts only epoch seconds in 1e9..2e9, repairs ms/us/ns and 1601-epoch values, and rejects the rest (l.388-407).
     - Duplicates are dropped by an MD5 `fields_hash` over the indexed fields, with a unique index (`heartbeat.rb` l.54-70).
     - Bulk posts are capped at 100 (`MAX_BULK_HEARTBEATS`).
     - Rack::Attack has a heartbeat throttle of 360 per minute per IP, but `Rack::Attack.enabled = false` (`config/initializers/rack_attack.rb` l.4), so it is off as written.
     - It records IP, a JA4 TLS fingerprint and machine name, which are available to admins.
     - I found no impossible-activity check such as keystroke rate or parallel sessions.

3. **API for a third-party tracker.**
   - `POST /api/hackatime/v1/users/:id/heartbeats` and `.bulk` (`config/routes.rb` l.352-360; controller `app/controllers/api/hackatime/v1/hackatime_controller.rb`). Also GET `users/:id/summaries`, `statusbar/today` and `users/current/stats/last_7_days`.
   - Auth: an API key sent as Bearer, Basic (base64-decoded) or `?api_key=` (`set_user`; `user_api_authentication.rb` l.8-38). Red users can still post (`allow_restricted: true`).
   - Accepted fields (`HEARTBEAT_KEYS`, controller l.8-13): entity, type, category, time, project, language, branch, editor, plugin, user_agent, is_write, lines, lineno, cursorpos, line_additions, line_deletions, machine, operating_system, project_root_count and created_at.
   - AI fields: ai_model, ai_session, ai_subscription_plan, ai_input_tokens, ai_output_tokens, ai_prompt_length, ai_line_changes and human_line_changes. These are included in the dedupe hash.
   - Category "ai coding" is split out in `durations_by_day`. Whether the UI shows it separately was not checked.
   - The editor is parsed from the user agent. Entity `test.txt` is stored as `test_entry`.

4. **How Hack Club programs consume it.** The repo shows only the mechanism. OAuth apps with the `profile` scope read `trust_factor`. The `admin` scope needs HQ approval (`docs/oauth/oauth-apps.md` l.27, 44). Project attribution is by project name plus an optional repo mapping (`ProjectRepoMapping`). I found no approval or YSWS (Hack Club's "You Ship, We Ship" programs) logic. `ysws_program` is an ignored column. How a program decides to approve hours is UNVERIFIED and lives outside this repo.

5. **License.** `LICENSE` is MIT, Copyright 2026 Hack Club. MIT code can be copied into an AGPL-3.0 project if the copyright and permission notice is kept. The reverse direction is not allowed. The algorithm (capped gaps) is simple enough to reimplement anyway. Add a notice only if code is copied verbatim.

6. **Where Tally disagrees.**
   - Tally credits per action: decisions and edits. Hackatime credits 2-minute-capped gaps between heartbeats.
   - Tally's heartbeat listener turns heartbeats into events and drops agent-written edits after a 15 s hold. That makes it stricter than Hackatime on agent edits, since Hackatime counts an "ai coding" category itself.
   - Hackatime would count a long agent run as active time if heartbeats keep arriving, and Tally would not.

## Recommendation

- Do not emit heartbeats to Hackatime. It would blur the human/agent line, and `docs/adapters/heartbeat.md` says Tally never contacts wakatime.com. If it is ever wanted, make it opt-in and send only human-only events as category "coding".
- Do not claim or compute a trust factor. It is a manual admin label. If a reviewer's Hackatime account is green, that tells them nothing about Tally's number.
- Publish a side-by-side figure. For the same window, give Tally's human hours next to a Hackatime-style number: a 120 s capped-gap sum over the same human events. Reviewers will compare Tally's total against the Hackatime coding total, and a gap is expected and explainable (Tally is smaller when agents are active).
- Match Hackatime's conventions in docs: 120 s cap, first heartbeat 0, ties by id.
- Avoid inflated numbers. Never sum per-project totals as if they were the user total.

## Not verified

- I did not search the whole repo for automatic ban rules.
- I did not check the UI for AI/human time display.
- I did not check the WakaTime upstream algorithm.
- How YSWS programs read Hackatime data is outside the repo.
- I did not check whether the rate limit is enabled in production, since `enabled = false` is set in the initializer.
