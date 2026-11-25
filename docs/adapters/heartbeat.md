# Editor heartbeats (WakaTime protocol)

Status: supported. Verified against **wakatime-cli v2.28.4**, the engine of the VS Code WakaTime extension, on Windows (requests, key format, delays: `crates/tally-daemon/tests/fixtures/heartbeat/NOTES.md`). Other plugins, macOS and Linux have not been observed.

Tally runs a small HTTP server on loopback that speaks the WakaTime heartbeat API. Editor plugins post to it and each accepted heartbeat becomes a `user_edit` event for the project that contains the file.

## Setup

1. It is on by default; set `enabled = false` in `config.toml` to turn it off. The settings:

   ```toml
   [heartbeat]
   enabled = true
   port = 25930          # default
   # token_file = "..."  # default: <tally home>/heartbeat.token
   ```

2. Run `tally heartbeat-config`. It creates the token file on first use, prints the same token every time, and prints three lines:

   ```
   api_url = http://127.0.0.1:25930/api
   api_key = <token, a UUID>
   heartbeat_rate_limit_seconds = 0
   ```

3. Put those three lines under `[settings]` in `~/.wakatime.cfg` (the file every WakaTime editor plugin reads), then restart the editor. This replaces any wakatime.com or Hackatime settings in that file: a stock plugin sends to one place only.

## What to know

- The server binds 127.0.0.1 only and rejects a foreign `Host` header. Other local programs cannot post without the token (Basic auth with the key, `Bearer`, or `?api_key=`); the token file is created with mode 0600 on unix. The token is a random UUID because wakatime-cli refuses any other key shape; a token file written by an older build (64 hex digits) is replaced the first time the daemon or `tally heartbeat-config` runs, so run that command again and update `~/.wakatime.cfg`.
- Only hashes leave the plugin payload. Tally keeps `hash(project id, relative path)`, line counts and timing; the file path, branch and language are dropped, and no path is written to the database.
- Heartbeats are held for 15 seconds before they become events, so agent file writes can be matched and filtered out first (an edit made by Claude Code is not credited as a human edit). If the daemon crashes, up to the last 15 seconds of held heartbeats are lost; that is the ceiling.
- The client talks to `<api_url>/users/current/heartbeats.bulk` (no `/v1`); the daemon also accepts `/v1/...` and the Hackatime prefix.
- A heartbeat is only accepted if it arrives within 3 minutes of its own timestamp (plus the 15 s hold); an older one is acknowledged to the plugin but counted as `late_dropped`, and the plugin does not retry. The real client can be slow: by default it sends one request per 120 s and queues the rest (the `heartbeat_rate_limit_seconds = 0` line removes that), and on one machine it sat for 73 s scanning other tools' AI logs before its first request. Finalization trails real time by about 3 minutes more while this source is enabled. Keep `[run] interval_secs` at 60 or less (default 15).
- A stock WakaTime client also posts heartbeats for other AI tools it finds on the machine (category `ai coding`, entity `<tool> <session id>`, type `app`, with token counts and project names), thousands on the first run. They are counted as `not_a_file` or `late_dropped` and never stored.
- `tally doctor` prints the counters: `accepted`, `dropped_agent`, `outside_project`, `late_dropped`, `not_a_file`.
- Tally never contacts `wakatime.com`. A plugin sends to one place only, so users who also want wakatime.com keep a second plugin configuration; that is out of scope.
