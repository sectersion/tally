# Codex adapter

**Status: unsupported, spike not run.** Tally does not read Codex logs yet. No log format has been verified, because the spike needs a logged-in Codex session and none was available.

## Where logs are believed to live (UNVERIFIED)

| OS | Path |
|----|------|
| Linux / macOS | `~/.codex/sessions/YYYY/MM/DD/rollout-<timestamp>-<id>.jsonl` (UNVERIFIED) |
| Windows | `%USERPROFILE%\.codex\sessions\...` (UNVERIFIED) |
| Override | `$CODEX_HOME` replaces `~/.codex` (UNVERIFIED) |

On the one machine checked, `~/.codex` held no sessions, so nothing could be confirmed.

## Expected signals (honesty table)

All entries are expectations, not measurements.

| Signal | Expected | Confidence |
|--------|----------|------------|
| Session start/end times | probably from event timestamps | UNVERIFIED |
| Active time vs idle | approximate at best; depends on per-event timestamps | UNVERIFIED |
| Project / repo | probably from the session cwd | UNVERIFIED |
| Token / cost data | possibly present; not used for time | UNVERIFIED |
| Session end marker | unknown; may need an idle timeout | UNVERIFIED |
| Format stability | not a public contract; may break between versions | UNVERIFIED |

Until the spike is run, Codex time is not tracked. See `crates/tally-daemon/tests/fixtures/codex/NOTES.md` for the steps to run it.
