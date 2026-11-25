# WakaTime heartbeat protocol notes

STATUS: OBSERVED for **wakatime-cli v2.28.4** (the engine inside the VS Code WakaTime extension 30.2.1), run on Windows 11 on 2026-11-24 against a capture listener and against the real daemon. Not observed: JetBrains, Vim, Sublime or any other plugin, macOS or Linux, wakatime-cli on other versions. Hackatime's own routes (`/api/hackatime/v1/...`) are accepted but never captured.

How it was done: `~/.wakatime/wakatime-cli-windows-amd64.exe` with `--config <scratch file>` and `WAKATIME_HOME=<scratch dir>`, so the user's real `~/.wakatime.cfg`, key and offline queue were never read or changed. A ten-line Python listener in a scratch directory (not in the repo) logged every request and answered with the response shapes below. Raw captures contained the machine's other AI sessions (see "Privacy") and were deleted; the fixtures keep scrubbed or synthetic values in the observed shapes.

## OBSERVED

- **Path:** `POST <api_url>/users/current/heartbeats.bulk`, with `api_url` taken as written. With `api_url = http://127.0.0.1:25930/api` the request is `/api/users/current/heartbeats.bulk`: **no `/v1`**. The earlier assumption (`/api/v1/users/current/...`) was wrong; the daemon accepts both now (`/api`, `/api/v1`, `/api/hackatime/v1`). Only bulk was seen; no single `heartbeats` request and **no GET of any kind**.
- **Batching:** 9 or 10 heartbeats per request, always an array. Heartbeats were never sent twice after a 201 (0 of 3439 repeated).
- **Auth:** `Authorization: Basic base64(api_key)` with the key alone (no `user:`). No `?api_key=`, no Bearer.
- **Headers:** `Host: 127.0.0.1:<port>` (port included), `Content-Type: application/json`, `Accept: application/json`, `Accept-Encoding: gzip` (the request body was not gzipped), `User-Agent: wakatime/v2.28.4 (windows-10.0.26300.9974-x86_64) go1.27.3 vscode/1.99.0 vscode-wakatime/30.2.1`, plus `Timezone` and `X-Machine-Name` (the machine's host name: never stored).
- **API key format:** the client refuses to send anything when the key is not UUID-shaped: `invalid api key format`, exit code 104, the heartbeat goes to its offline queue. The daemon's old 64-hex token therefore never worked with a stock client. The daemon now generates a version-4 UUID and replaces an old non-UUID token file once.
- **Fields seen:** `entity`, `type` (`file` and `app`), `time` (float seconds; an integer for some), `category`, `project`, `branch`, `language`, `is_write`, `lineno`, `cursorpos`, `lines`, `project_root_count`, `dependencies` (array), `user_agent`, and the AI fields `ai_session`, `ai_prompt_length`, `ai_input_tokens`, `ai_output_tokens`, `ai_cached_input_tokens`, `ai_line_changes`. **`ai_line_changes` can be negative** (minimum seen -51): the daemon's unsigned field rejected such heartbeats; it is signed now. `human_line_changes`, `line_additions` and `line_deletions` were not seen; they are signed as well.
- **Responses:** a bulk `201` with `{"responses":[[{"data":{...}},201],...]}` satisfied the client. It did not resend and exited 0.
- **Server errors:** a `404` for every request gave exit codes 102 and 112 and the heartbeats stayed in the client's offline queue (`offline_heartbeats.bdb`, retried on later runs). The client does not give up or crash.
- **Rate limit:** by default the client sends at most one request per 120 s and queues the rest; three runs one second apart delivered one heartbeat. `heartbeat_rate_limit_seconds = 0` under `[settings]` in the config file turns this off (three of three delivered). `tally heartbeat-config` prints that line.
- **Delay:** on the dev machine the first request left 73 s after the client started (it scans other tools' AI logs first, including a `\\wsl$` path that is not readable), so a heartbeat arrived 86 s after its own `time`. The daemon used to drop anything more than 75 s old; the allowance is now 180 s (`HEARTBEAT_LATE_MS`), and the heartbeat watermark trails real time by 195 s.

## Privacy: what a stock client also sends

wakatime-cli v2.28.4 reads other AI tools' local logs itself and posts them as heartbeats with category `ai coding`: type `app`, entity `"<Tool> <session id>"`, the project name, token counts and prompt lengths, and thousands of old ones on the first run (3,439 in one capture). Anything pointed at Tally's daemon receives them. The daemon counts type `app` as `not_a_file`, old ones as `late_dropped`, and ones outside a registered repository as `outside_project`; none of it is stored (the heartbeat source keeps only a hash of the project id and relative path). Users who send to wakatime.com or Hackatime with the same client send the same data there.

## Still unknown

- Whether other plugins use single `heartbeats` calls or GET routes (status bar, `users/current`).
- Behaviour with a gzip request body, an `Expect: 100-continue`, or IPv6 `localhost`.
- Plugin-specific rate limits (the VS Code extension starts the client per event; the `--heartbeat-rate-limit-seconds` default of 120 applies unless the config file overrides it).
- The same on macOS and Linux.

## Fixtures

- `single.json`, `bulk.json`: hand-written, kept for the parser tests.
- `wakatime-cli-2.28.4-bulk.json`: observed shapes, scrubbed (paths replaced with `C:/tallyproj/...`, project names replaced, the session id fake) and with synthetic values for the AI heartbeats; the negative `ai_line_changes` value is from the observed range.
- `wakatime-cli-requests.txt`: the request line and headers as observed, host name replaced.
- `vscode-request.txt`: the request the Tally VS Code extension (tally-vscode/0.1.0) sent from inside VS Code 1.147.0, OBSERVED on Windows 11 on 2026-11-24 in a scratch profile against a scratch listener: a single (not bulk) `POST /api/users/current/heartbeats` on opening a file (active-editor change), key removed. Against the real scratch daemon the same open counted `not_a_file=1` (read, no line change; by design), and an external edit to the open file (text-change event) counted `accepted=1`, `late_dropped=0`. NOT observed: a save event from a human keypress (no GUI automation; the save path is only unit-tested), any other VS Code fork, macOS, Linux.
