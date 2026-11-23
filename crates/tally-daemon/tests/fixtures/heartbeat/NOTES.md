# WakaTime heartbeat protocol notes

STATUS: SYNTHETIC / UNVERIFIED. No real WakaTime plugin or wakatime-cli was run against a listener. Everything below comes from the documented WakaTime API, and the fixtures here are hand-written to match. Before relying on it, do the capture in plan 08 Task 16 steps 1-2 (listener script outside the repo, `api_url = http://127.0.0.1:<port>/api` in `~/.wakatime.cfg`) and replace this file with observations.

## Assumed (unverified)
- Paths: `POST <api_url>/v1/users/current/heartbeats` (one object) and `.../heartbeats.bulk` (array, max 25). wakatime-cli normally sends bulk and replays its offline queue the same way. Hackatime serves the same routes under `/api/hackatime/v1/`.
- Auth: `Authorization: Basic <base64(api_key)>` (key alone, no `user:`), or `?api_key=`; `Bearer <key>` for OAuth tokens.
- Fields: entity, type (file/app/domain), time (float seconds), category, project, branch, language, is_write, lines, lineno, cursorpos, line_additions, line_deletions, ai_line_changes, human_line_changes, user_agent, plugin.
- Responses: single 201 `{"data":{"id","entity","type","time"}}`; bulk 201/202 `{"responses":[[{"data":{...}},201],...]}`, one pair per input, in order; 400 for malformed JSON. A non-2xx makes the client queue offline, so an accepted-and-dropped heartbeat must still answer 201.

## Unknown until captured
- GET calls (status bar, users/current) and what a 404 does to the plugin.
- Whether Host carries the port, exact Content-Type and User-Agent, and whether the client resends after our 201 shape.
