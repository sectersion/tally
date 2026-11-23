# Aider adapter

Status: unsupported, spike not run. tally does not read aider data yet.

## Where logs are believed to live (all UNVERIFIED on every OS)
Per vendor docs, aider writes bare-filename defaults, so presumably in the project directory (not a per-OS global dir):
- `.aider.chat.history.md` (default)
- `.aider.input.history` (default)
- `.aider.llm.history` (opt-in via `--llm-history-file`)

| OS | Location |
|---|---|
| Windows | project dir, UNVERIFIED |
| macOS | project dir, UNVERIFIED |
| Linux | project dir, UNVERIFIED |

## Expected signals (honesty table)
| Signal | Expectation |
|---|---|
| Session start/end | UNVERIFIED, format unknown |
| Activity timestamps | UNVERIFIED |
| Token usage | UNVERIFIED, maybe only in opt-in llm history |
| Project attribution | likely by directory, UNVERIFIED |

See `crates/tally-daemon/tests/fixtures/aider/NOTES.md` for sources and the steps to run the real spike.
