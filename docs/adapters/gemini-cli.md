# Gemini CLI adapter

Status: **unsupported, spike not run.** Tally does not read Gemini CLI data yet. Its log format has not been inspected, and the adapter will not be built from guesses.

## Where logs are believed to live (all UNVERIFIED)
| OS | Believed path |
|---|---|
| Linux | `~/.gemini/tmp/<project_hash>/chats/` |
| macOS | `~/.gemini/tmp/<project_hash>/chats/` |
| Windows | `%USERPROFILE%\.gemini\tmp\<project_hash>\chats\` |

Source: https://geminicli.com/docs/cli/session-management/ (documents the Unix path only; the Windows path is an assumption).

## Expected signals
| Signal | Expected | Status |
|---|---|---|
| Session start/end | Session files with timestamps | UNVERIFIED |
| Active time | Message/tool timestamps | UNVERIFIED |
| Project | Hash of project root, not the path itself | UNVERIFIED |
| Tokens | Input/output/cached counts | UNVERIFIED (documented, format unknown) |
| Tool calls | Recorded with inputs/outputs | UNVERIFIED |

Until the spike in `crates/tally-daemon/tests/fixtures/gemini-cli/NOTES.md` is run, nothing here should be relied on.
