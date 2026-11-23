# Editor heartbeats (WakaTime protocol)

Status: supported, but the protocol notes are SYNTHETIC / UNVERIFIED: no real editor plugin was captured against the listener (see `crates/tally-daemon/tests/fixtures/heartbeat/NOTES.md`).

Tally runs a small HTTP server on loopback that speaks the WakaTime heartbeat API. Editor plugins post to it and each accepted heartbeat becomes a `user_edit` event for the project that contains the file.

## Setup

1. Enable it in `config.toml`:

   ```toml
   [heartbeat]
   enabled = true
   port = 25930          # default
   # token_file = "..."  # default: <tally home>/heartbeat.token
   ```

2. Run `tally heartbeat-config`. It creates the token file on first use, prints the same token every time, and prints two lines:

   ```
   api_url = http://127.0.0.1:25930/api
   api_key = <token>
   ```

3. Put those two lines under `[settings]` in `~/.wakatime.cfg` (the file every WakaTime editor plugin reads), then restart the editor.

## What to know

- The server binds 127.0.0.1 only and rejects a foreign `Host` header. Other local programs cannot post without the token (Basic auth with the key, `Bearer`, or `?api_key=`); the token file is created with mode 0600 on unix.
- Only hashes leave the plugin payload. Tally keeps `hash(project id, relative path)`, line counts and timing; the file path, branch and language are dropped, and no path is written to the database.
- Heartbeats are held for 15 seconds before they become events, so agent file writes can be matched and filtered out first (an edit made by Claude Code is not credited as a human edit). If the daemon crashes, up to the last 15 seconds of held heartbeats are lost; that is the ceiling.
- `tally doctor` prints the counters: `accepted`, `dropped_agent`, `outside_project`, `late_dropped`, `not_a_file`.
- Tally never contacts `wakatime.com`. A plugin sends to one place only, so users who also want wakatime.com keep a second plugin configuration; that is out of scope.
