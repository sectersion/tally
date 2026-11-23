Layout: `<harness>/<version>/` holds one scrubbed log plus `expected.json` (and `NOTES.md` for real adapters).
Scrub rule: no prompt text, paths, names or secrets; keep only the record structure and replaceable filler words.
Add a version: drop the log in a new `<version>/` dir, run `TALLY_BLESS=1 cargo test -p tally-daemon --features test-kit --test golden`, and review the `expected.json` diff.
