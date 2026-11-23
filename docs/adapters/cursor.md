# Cursor adapter

**Status: unsupported, spike not run.** tally does not read Cursor data today. Nothing below has been verified on a real install.

## Where logs are believed to live (UNVERIFIED)

| OS | Path |
|---|---|
| Windows | `%APPDATA%\Cursor\User\` |
| macOS | `~/Library/Application Support/Cursor/User/` |
| Linux | `~/.config/Cursor/User/` |

Inside: `workspaceStorage/<hash>/state.vscdb` (per workspace) and `globalStorage/state.vscdb` (message bodies), both SQLite. Source is third-party and reverse-engineered, not vendor documentation: https://www.mantyl.dev/guides/cursor-chat-history-location

## Expected signals (honesty table)

| Signal | Expected | Confidence |
|---|---|---|
| Session start/end | Derivable from conversation creation time and last message time | UNVERIFIED |
| Per-message timestamps | `createdAt` on bubble rows | UNVERIFIED (format unknown) |
| Project/workspace | `workspace.json` folder URI | UNVERIFIED |
| Live heartbeat | Unknown; DB is SQLite and may be WAL-buffered | UNVERIFIED |
| Tokens / cost | Not known to be recorded | UNVERIFIED |
| Schema stability | Undocumented; may break on Cursor updates | Likely fragile |

See `crates/tally-daemon/tests/fixtures/cursor/NOTES.md` for sources and the steps needed to run the real spike.
