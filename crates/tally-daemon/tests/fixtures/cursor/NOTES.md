# Cursor spike notes

Status: **spike NOT run.** Docs-only research; Cursor was not installed or run. No fixture and no parser exist, and none may be written from this file alone.

Marking: OBSERVED = read in a third-party source (not verified on a real machine by us). UNVERIFIED = inferred or unconfirmed. Nothing here is first-hand.

## Claims

1. Cursor is a VS Code fork; user data dir is `%APPDATA%\Cursor\User` (Win), `~/Library/Application Support/Cursor/User` (macOS), `~/.config/Cursor/User` (Linux). OBSERVED (third-party): https://www.mantyl.dev/guides/cursor-chat-history-location
2. `workspaceStorage/<hash>/` has `workspace.json` (`folder` = file URI, percent-encoded on Windows) and `state.vscdb` (SQLite). OBSERVED (third-party): same URL.
3. Workspace DB table `ItemTable`, key `composer.composerData`, JSON with `allComposers[]` (composerId, name, creation timestamp; exact field name unknown). OBSERVED (third-party): same URL.
4. `globalStorage/state.vscdb`, table `cursorDiskKV`, rows `bubbleId:<composerId>:<bubbleId>`; JSON has `type` (1 user, 2 model), `text`, `createdAt`. OBSERVED (third-party): same URL.
5. Timestamp format of `createdAt` (ms epoch vs ISO). UNVERIFIED.
6. Schema is undocumented and reverse-engineered; may change between Cursor versions. OBSERVED (same URL); no vendor doc found.
7. DB is held open by Cursor; read-only/copy needed; global DB can exceed 1 GB. OBSERVED (same URL).
8. Whether per-message timestamps exist for all bubbles (tool-only rows, agent mode). UNVERIFIED.
9. Whether `state.vscdb` writes are flushed promptly (WAL) enough for near-live heartbeats. UNVERIFIED.
10. Other candidate sources (not read): https://seanthegeek.net/doubleagent/research/cursor.html , https://groundedby.com/agents/cursor , https://github.com/farique/cursor-chat-explorer . UNVERIFIED (not fetched).
11. Cursor hooks/extension API as a cleaner signal source. UNVERIFIED (not researched).

## Maintainer: how to run the real spike

1. Install Cursor, sign in, open a throwaway workspace, run one chat and one agent session with a tool call, then close it.
2. Copy (do not open live) `workspaceStorage/<hash>/state.vscdb` and `globalStorage/state.vscdb`; also note any `-wal` files.
3. `sqlite3 <copy> ".schema"`, then dump `composer.composerData` and a few `bubbleId:*` rows; confirm claims 3-5, 8, 9 on each OS you support.
4. Redact secrets/paths, save minimal samples as fixtures in this directory, record Cursor version.
5. Decide: poll mtime of the DBs only (activity signal), or parse bubbles. Then update docs/adapters/cursor.md and replace UNVERIFIED marks.
