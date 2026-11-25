# Code worth taking from the other entries

All the readable repos are **AGPL-3.0**, the same as Tally, so porting with attribution is legal (the `LICENSE` files of Sablier, Steer, StruggleTime and Kairos were all checked and are the GNU AGPL v3; Hindsight's repo is not public, so its licence is only the site's claim). Almost all of it is Rust or Go and written for a different event model, so the real job is "read the same fields, emit our `Kind`", not copy-paste.

**Rule that still applies:** Tally's adapter docs say adapters are "not built from guesses". A ported parser is a hypothesis until it has been checked against a real session with a structure-only script, the way `crates/tally-daemon/tests/fixtures/claude_code/FORMAT.md` was produced.

| Take | From | Size | Why | Caveat |
|---|---|---|---|---|
| Harness parsers for Codex, Cursor, Gemini CLI, Copilot CLI, opencode, Cline, Qwen, Amp and others | Kairos `crates/core/src/sources/*.rs` | ~9.5k lines total. `codex.rs` 632 (12 tests, 27 inline sample lines), `cursor.rs` 907 (4 tests), `gemini.rs` 395 (4 tests), `copilot_cli.rs` 416 (5 tests), `claude.rs` 941 (26 tests), `opencode.rs` 1051 | Tally reads only Claude Code and Copilot CLI. Codex, Cursor, Gemini and Aider are all "unsupported, spike not run" | Emits Kairos's `Kind::Prompt{words,pasted,..}`, not ours. Cursor and Gemini have thin tests and little sample data |
| Codex injected-context list (`<environment_context>`, `<user_instructions>`, `# AGENTS.md instructions`, `<turn_aborted>`, subagent notifications, ...) | Kairos `sources/codex.rs` `INJECTED` | ~10 lines | Ready-made filter list for the Codex adapter | Verify against a real session |
| Claude parser as a cross-check | Kairos `sources/claude.rs` | 941 lines | Diffing it against our `line.rs` + `session.rs` would surface drift like the compaction-summary bug | Different event model |
| Claude/Codex/Copilot/OpenCode readers | Steer `src/claude.rs`, `codex.rs`, `copilot.rs`, `opencode.rs` | small | Readable Rust, a quick start for the Codex spike. `claude.rs::user_event` has the injected-entry filters | Steer says its Copilot and OpenCode readers were written from docs and not tried on real use |
| Paste unwrapping for Claude Code 2.1.x | Kairos `core/src/parse.rs::unwrap_pastes` | ~40 lines | Matches `<pasted_content id="…">…</pasted_content id="…">` seen in real logs | **Do not copy Steer's `typed_part`**: it expects a closing tag without the id |
| Prompt "shape" counts (indentation, log-line ratio) with no text kept | Kairos `core/src/shape.rs` | 193 lines | Count-only text features, same privacy posture as ours | Check against the README privacy list before adding any feature |
| Persona simulator | Kairos `crates/sim` | not measured | Grow our `synthetic()` into named attack personas for plan 12's red-team | |
| Sweep-line overlap attribution | Sablier `internal/credit/credit.go` | Go | We already have `spans.rs` `union/subtract/clip/total_ms` | Rewrite only if our merge proves wrong |
| PTY escape-sequence counter (bracketed paste, focus in/out, mouse) | Sablier `internal/launcher/input.go` + `input_test.go` | Go, small | Only if we build the optional wrapper | Rewrite in Rust |
| Pure `Map(state, hook) -> (state, events)` test style | Sablier mappers | | Already the shape of our `classify` / `Session::feed` | No change |

## Skip
- Kairos's WakaTime config takeover (`agent/src/wakatime.rs`): two daemons rewriting `~/.wakatime.cfg` will fight.
- StruggleTime's daemon: file-change bursts as the agent/human split, unkeyed chain, unsigned buckets.
- Sablier's hook injector (`--settings` merge): clashes with user settings.
- Steer's English word-list classifier: gameable, duplicated in two languages.
- Anything that uploads prompt text to a third-party model (Sablier `verify`).

## Process for taking a parser
1. Read the Kairos parser for the harness; list the log paths and field names it uses.
2. Write them into `docs/adapters/<harness>.md` under the UNVERIFIED heading, with the source cited.
3. Run a structure-only script (see `datamining/scripts/`) over a real session of that harness; record counts and enum-like values only, never content.
4. Only then write the adapter, with fixtures hand-written to the measured shape.
