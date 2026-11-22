# Scoring Crate Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A pure Rust crate, `tally-scoring`, that turns a list of events into hash-chained claims exactly as `docs/spec/scoring.md` defines, usable natively by the daemon and as WebAssembly by the server.

**Architecture:** One library crate with no I/O. `score(&[Event]) -> Vec<Claim>` is a pure function; ledger hashing and text features are separate modules. Text features (zstd, tokenizing) sit behind a `text` cargo feature because only the daemon ever sees text; the `wasm` feature exposes JSON-in/JSON-out bindings for the server.

**Tech Stack:** Rust stable (edition 2021), `serde` + `serde_json`, `sha2`, `hex`, `zstd` (feature `text`), `wasm-bindgen` (feature `wasm`), `wasm-pack`, Bun for the WASM smoke test.

**Spec:** `docs/spec/scoring.md` (ruleset `2026.11-0`). Read it before every task.

## Global Constraints

- Ruleset string is exactly `2026.11-0`; every constant from spec §7 lives in `ruleset.rs` with the spec's name and value.
- All time math in integer milliseconds (`i64`); output seconds are `ms / 1000`, floored. No floats anywhere in scoring or hashing.
- `score` is deterministic: same events in any input order produce identical output (sort by `(ts, id)` first).
- No I/O, no clock reads, no randomness in the crate.
- Default features compile to `wasm32-unknown-unknown` (no `zstd` in default features).
- License header not required per file; crate `license = "AGPL-3.0-only"`.

## Review Focus

1. **Unsorted or duplicate input:** events arrive out of order or with a repeated `id`. Expected: sorted by `(ts, id)`; duplicates by `id` dropped, first kept. Test in Task 4.
2. **Clock going backwards:** two actions where the later one has a smaller `ts` after a system clock change. Expected: after sorting, no negative interval; a zero-length interval yields a zero claim. Test in Task 4.
3. **Overlapping spans:** idle and agent-busy spans overlap each other or the protected windows. Expected: subtraction is a union, never double-counted. Test in Task 2.
4. **Spans crossing the interval edge:** an idle span that starts before `t_prev` or ends after `t`. Expected: clipped to the interval; grace measured from the span's real start. Test in Task 2.
5. **Action with `project = null`:** expected: a claim with `project: null`, counted but unattributed. Test in Task 4.

## File Structure

```
Cargo.toml                         workspace root (members = ["crates/*"])
crates/tally-scoring/
  Cargo.toml
  src/lib.rs                       module wiring + re-exports
  src/ruleset.rs                   RULESET_VERSION + all spec §7 constants
  src/event.rs                     Event, Kind and enums (spec §2)
  src/spans.rs                     interval-set arithmetic
  src/caps.rs                      cap() and fail→pass detection (spec §4)
  src/score.rs                     score() (spec §3)
  src/ledger.rs                    LedgerEntry, canonical JSON, hash chain (spec §6)
  src/text.rs                      relevance_pm, novelty_bytes (feature "text", spec §5)
  src/wasm.rs                      wasm-bindgen exports (feature "wasm")
  tests/scenarios.rs               golden end-to-end scenarios
wasm-smoke/smoke.test.ts           Bun test against the wasm-pack output
.github/workflows/scoring.yml      CI
```

---

### Task 1: Workspace, ruleset, and event types

**Files:**
- Create: `Cargo.toml`, `crates/tally-scoring/Cargo.toml`, `src/lib.rs`, `src/ruleset.rs`, `src/event.rs`

**Interfaces:**
- Produces:
  - `pub const RULESET_VERSION: &str = "2026.11-0";` and one `pub const NAME: i64` per spec §7 constant, in milliseconds where it is a duration (e.g. `IDLE_GRACE_MS: i64 = 180_000`), plain values otherwise (`READ_CPS: i64 = 25`, `NOVELTY_MIN: u32 = 24`, `VOCAB_MAX: usize = 50_000`, `THINK_RELEVANCE_PM: u16 = 500`). Also `TRIVIAL_CHARS: u32 = 20`, `TRIVIAL_RELEVANCE_PM: u16 = 150`, `INTERRUPT_FOLLOW_MS: i64 = 120_000`.
  - `pub struct Event { pub v: u32, pub id: String, pub ts: i64, pub source: String, pub project: Option<String>, pub session: Option<String>, #[serde(flatten)] pub kind: Kind }`
  - `#[serde(tag = "kind", rename_all = "snake_case")] pub enum Kind { Prompt { chars: u32, relevance_pm: u16, novelty_bytes: u32 }, Interrupt, ToolDecision { decision: Decision }, UserEdit { file_hash: String, lines_changed: u32 }, TestRun { cmd_hash: String, exit_code: i32, initiator: Actor }, Commit { sha: String, by: Actor, lines_changed: u32 }, AgentBusy { end_ts: i64 }, Idle { end_ts: i64 }, AgentOutput { chars: u32 } }`
  - `pub enum Decision { Approve, Deny }`, `pub enum Actor { User, Agent }` (both `snake_case` serde).
  - `impl Event { pub fn is_human_action(&self) -> bool }`: true for Prompt, Interrupt, ToolDecision, UserEdit, `TestRun{initiator: User}`, `Commit{by: User}`.

- [ ] **Step 1: Write the failing tests** in `src/event.rs`:

```rust
#[test]
fn prompt_roundtrips_with_flat_kind() {
    let json = r#"{"v":1,"id":"01A","ts":1000,"source":"claude-code","project":"p","session":"s","kind":"prompt","chars":40,"relevance_pm":600,"novelty_bytes":90}"#;
    let e: Event = serde_json::from_str(json).unwrap();
    assert!(matches!(e.kind, Kind::Prompt { relevance_pm: 600, .. }));
    assert_eq!(serde_json::to_string(&e).unwrap(), json);
}

#[test]
fn human_action_classification() {
    // agent test runs and agent commits are not human actions; spans are not
    assert!(!ev(Kind::TestRun { cmd_hash: "x".into(), exit_code: 1, initiator: Actor::Agent }).is_human_action());
    assert!(ev(Kind::TestRun { cmd_hash: "x".into(), exit_code: 1, initiator: Actor::User }).is_human_action());
    assert!(!ev(Kind::Commit { sha: "a".into(), by: Actor::Agent, lines_changed: 5 }).is_human_action());
    assert!(!ev(Kind::Idle { end_ts: 5 }).is_human_action());
}
```

`ev(kind)` is a `#[cfg(test)]` helper building an `Event` with `v:1, id:"x", ts:0, source:"test", project:None, session:None`.

- [ ] **Step 2:** Run `cargo test -p tally-scoring`. Expected: FAIL (types not defined).
- [ ] **Step 3:** Implement `ruleset.rs` and `event.rs` with the interfaces above. Field order in `Event` must match the JSON in the test so serialization roundtrips byte-for-byte.
- [ ] **Step 4:** Run `cargo test -p tally-scoring`. Expected: PASS.
- [ ] **Step 5:** Commit: `feat(scoring): ruleset constants and event types`.

---

### Task 2: Span arithmetic

**Files:**
- Create: `src/spans.rs`

**Interfaces:**
- Produces:
  - `pub struct Span { pub start: i64, pub end: i64 }` (half-open `[start, end)`, ms).
  - `pub fn union(spans: &[Span]) -> Vec<Span>`: sorted, merged, empty spans removed.
  - `pub fn subtract(a: &[Span], b: &[Span]) -> Vec<Span>`: parts of `a` not covered by `b`.
  - `pub fn clip(spans: &[Span], window: Span) -> Vec<Span>`
  - `pub fn total_ms(spans: &[Span]) -> i64`

- [ ] **Step 1: Write the failing tests:**

```rust
#[test] fn union_merges_overlaps() {
    assert_eq!(union(&[s(0,10), s(5,20), s(30,40)]), vec![s(0,20), s(30,40)]);
}
#[test] fn subtract_handles_partial_and_full_cover() {
    assert_eq!(subtract(&[s(0,100)], &[s(10,20), s(50,200)]), vec![s(0,10), s(20,50)]);
}
#[test] fn overlapping_subtractions_not_double_counted() {
    let removed = union(&[s(10,60), s(40,90)]);
    assert_eq!(total_ms(&subtract(&[s(0,100)], &removed)), 20);
}
#[test] fn clip_to_window() {
    assert_eq!(clip(&[s(-50,30), s(80,500)], s(0,100)), vec![s(0,30), s(80,100)]);
}
```

`s(a,b)` is a test helper for `Span { start: a, end: b }`.

- [ ] **Step 2:** Run `cargo test -p tally-scoring spans`. Expected: FAIL.
- [ ] **Step 3:** Implement the four functions; sort by `start`, single linear pass each.
- [ ] **Step 4:** Run the same command. Expected: PASS.
- [ ] **Step 5:** Commit: `feat(scoring): span arithmetic`.

---

### Task 3: Caps and fail→pass detection

**Files:**
- Create: `src/caps.rs`

**Interfaces:**
- Consumes: `Event`, `Kind`, constants from Task 1.
- Produces:
  - `pub fn cap_ms(action: &Event, ctx: &CapContext) -> i64`
  - `pub struct CapContext { pub interrupt_followed_by_prompt: bool, pub completes_fail_pass_loop: bool }`
  - `pub fn completes_fail_pass_loop(events: &[Event], idx: usize) -> bool`: `events` sorted; true if `events[idx]` is a `TestRun` with `exit_code == 0` and an earlier `TestRun` (any initiator) with the same `cmd_hash` and `exit_code != 0` exists within `LOOP_WINDOW_MS`, with at least one human action strictly between them.
  - `pub fn is_trivial_prompt(chars: u32, relevance_pm: u16) -> bool`

Cap values (spec §4), in ms: trivial prompt 90 s; prompt `(120 + relevance_pm as i64 * 108 / 100)` s, max 1200 s; interrupt 120 s if followed, else 30 s; approve 60 s; deny 180 s; user edit 180 s; user test run 120 s, or 1200 s if it completes a loop; commit 60 s if `lines_changed < 3`, else 600 s. Non-actions return 0.

- [ ] **Step 1: Write the failing tests:**

```rust
#[test] fn prompt_caps() {
    let none = CapContext { interrupt_followed_by_prompt: false, completes_fail_pass_loop: false };
    assert_eq!(cap_ms(&prompt(9, 900), &none), 90_000);      // trivial by length
    assert_eq!(cap_ms(&prompt(200, 149), &none), 90_000);    // trivial by relevance
    assert_eq!(cap_ms(&prompt(200, 600), &none), 768_000);   // 120 + 648
    assert_eq!(cap_ms(&prompt(200, 1000), &none), 1_200_000);
}
#[test] fn interrupt_caps() {
    assert_eq!(cap_ms(&ev(Kind::Interrupt), &CapContext { interrupt_followed_by_prompt: true, completes_fail_pass_loop: false }), 120_000);
    assert_eq!(cap_ms(&ev(Kind::Interrupt), &CapContext { interrupt_followed_by_prompt: false, completes_fail_pass_loop: false }), 30_000);
}
#[test] fn fail_pass_loop_requires_human_action_between() {
    let fail = test_run_at(0, "x", 1); let pass = test_run_at(600_000, "x", 0);
    assert!(!completes_fail_pass_loop(&[fail.clone(), pass.clone()], 1));
    let p = prompt_at(300_000, 200, 600);
    assert!(completes_fail_pass_loop(&[fail, p, pass], 2));
}
#[test] fn fail_pass_loop_respects_window() {
    let v = [test_run_at(0, "x", 1), prompt_at(1_000, 200, 600), test_run_at(3_600_001, "x", 0)];
    assert!(!completes_fail_pass_loop(&v, 2));
}
```

Helpers: `prompt(chars, rel)`, `prompt_at(ts, chars, rel)`, `test_run_at(ts, cmd_hash, exit_code)` (user-initiated).

- [ ] **Step 2:** Run `cargo test -p tally-scoring caps`. Expected: FAIL.
- [ ] **Step 3:** Implement per the interface and cap values above.
- [ ] **Step 4:** Run the same command. Expected: PASS.
- [ ] **Step 5:** Commit: `feat(scoring): caps and fail-pass loop detection`.

---

### Task 4: `score()`

**Files:**
- Create: `src/score.rs`, `tests/scenarios.rs`

**Interfaces:**
- Consumes: `Event`/`Kind` (Task 1), `Span`/`union`/`subtract`/`clip`/`total_ms` (Task 2), `cap_ms`/`CapContext`/`completes_fail_pass_loop` (Task 3).
- Produces:
  - `pub fn score(events: &[Event]) -> Vec<Claim>`: one claim per human action, in time order.
  - `#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)] pub struct Claim { pub action_id: String, pub ts: i64, pub project: Option<String>, pub kind: String, pub verified_s: u32, pub unverified_s: u32, pub reasons: Vec<String>, pub features: BTreeMap<String, i64> }`
  - `kind` is the event's serde tag (`"prompt"`, `"user_edit"`, …).
  - `features`: prompts get `chars`, `relevance_pm`, `novelty_bytes`; `user_edit` and `commit` get `lines_changed`; `test_run` gets `exit_code`. Others empty.
  - `reasons`, in this order when they apply: `first_action`, `read_protected`, `think_protected`, `idle_subtracted`, `agent_busy_subtracted`, `fail_pass_loop`, `interrupt_followed`, `capped`.

Algorithm: spec §3 exactly. Implementation notes the spec leaves open:
1. Sort a copy by `(ts, id)`; drop later duplicates of an `id`.
2. Protected windows = union of reading windows (each `AgentOutput` at `ts`: `[ts, ts + min(chars * 1000 / READ_CPS, MAX_READ_MS))`) plus the thinking window `[t − THINK_MAX_MS, t)` when the action is a prompt with `relevance_pm ≥ THINK_RELEVANCE_PM`.
3. Removed = union of (idle part beyond `IDLE_GRACE_MS`, measured from each idle span's real `ts`) and (each `AgentBusy` span ∩ idle part beyond `AGENT_IDLE_MS`), then `subtract(removed, protected)`, then `clip` to the interval.
4. `raw_ms = interval length − total_ms(removed)`; `verified_ms = min(raw_ms, cap_ms)`; both floored to seconds at the end.
5. `interrupt_followed_by_prompt` looks ahead in the sorted list for a `Prompt` within `INTERRUPT_FOLLOW_MS`. The daemon treats claims newer than `INTERRUPT_FOLLOW_MS` as provisional (plan 2), so `score` stays pure.

- [ ] **Step 1: Write the failing golden scenarios** in `tests/scenarios.rs`. Times below are in seconds; helpers convert to ms. Every prompt has `chars = 200` unless noted.

```rust
#[test] fn lunch_break_earns_grace_plus_thinking_only() {
    // prompt rel 600 at 0; idle 60..1860; prompt rel 600 at 1900
    let c = score(&[prompt_at(0, 600), idle(60, 1860), prompt_at(1900, 600)]);
    assert_eq!((c[0].verified_s, c[0].unverified_s), (120, 0));
    assert_eq!((c[1].verified_s, c[1].unverified_s), (540, 0));
    assert!(c[1].reasons.contains(&"think_protected".into()));
}
#[test] fn watching_generation_earns_nothing_reading_does() {
    // prompt at 0; agent busy 5..305; idle 10..300; output 2500 chars at 305; prompt rel 300 at 420
    let c = score(&[prompt_at(0, 600), busy(5, 305), idle(10, 300), output(305, 2500), prompt_at(420, 300)]);
    assert_eq!(c[1].verified_s, 175);
    assert!(c[1].reasons.contains(&"agent_busy_subtracted".into()));
}
#[test] fn multitasking_splits_by_next_action() {
    let c = score(&[prompt_in(0, "A", 600), prompt_in(600, "B", 400), edit_in(700, "A")]);
    assert_eq!((c[1].project.as_deref(), c[1].verified_s, c[1].unverified_s), (Some("B"), 552, 48));
    assert_eq!((c[2].project.as_deref(), c[2].verified_s), (Some("A"), 100));
}
#[test] fn try_again_is_capped() {
    let c = score(&[prompt_at(0, 600), prompt_chars_at(300, 9, 600)]);
    assert_eq!((c[1].verified_s, c[1].unverified_s), (90, 210));
}
#[test] fn fail_pass_loop_gets_large_cap() {
    let c = score(&[test_at(0, "x", 1), prompt_at(300, 600), test_at(900, "x", 0)]);
    assert_eq!([c[0].verified_s, c[1].verified_s, c[2].verified_s], [120, 300, 600]);
    assert!(c[2].reasons.contains(&"fail_pass_loop".into()));
}
// Review Focus 1, 2, 5
#[test] fn order_and_duplicates_do_not_matter() {
    let a = [prompt_at(0, 600), idle(60, 1860), prompt_at(1900, 600)];
    let mut b = vec![a[2].clone(), a[0].clone(), a[1].clone(), a[0].clone()];
    assert_eq!(score(&a), score(&b));
    b.reverse(); assert_eq!(score(&a), score(&b));
}
#[test] fn same_timestamp_actions_get_zero() {
    let c = score(&[prompt_at(100, 600), edit_at(100)]);
    assert_eq!(c[1].verified_s + c[1].unverified_s, 0);
}
#[test] fn null_project_is_kept_unattributed() {
    let c = score(&[prompt_no_project_at(0, 600)]);
    assert_eq!((c[0].project.clone(), c[0].verified_s), (None, 120));
}
```

Helper ids must sort in creation order (e.g. `format!("{:06}", n)` from a counter) so duplicates are real duplicates.

- [ ] **Step 2:** Run `cargo test -p tally-scoring --test scenarios`. Expected: FAIL.
- [ ] **Step 3:** Implement `score` per the notes above.
- [ ] **Step 4:** Run `cargo test -p tally-scoring`. Expected: all PASS. If a golden value disagrees, recheck against spec §3 by hand before changing the test; the expected values were derived from the spec.
- [ ] **Step 5:** Commit: `feat(scoring): score() with golden scenarios`.

---

### Task 5: Ledger hash chain

**Files:**
- Create: `src/ledger.rs`

**Interfaces:**
- Consumes: `Claim` (Task 4), `RULESET_VERSION`.
- Produces:
  - `pub struct LedgerEntry { pub v: u32, pub ruleset: String, pub action_id: String, pub ts: i64, pub project: Option<String>, pub kind: String, pub verified_s: u32, pub unverified_s: u32, pub reasons: Vec<String>, pub features: BTreeMap<String, i64>, pub prev_hash: String, pub hash: String }` (spec §6; `v = 1`).
  - `pub const GENESIS_HASH: &str` = 64 `'0'` characters.
  - `pub fn canonical_json(e: &LedgerEntry) -> String`: the entry without `hash`, keys sorted, no whitespace. Build it with `serde_json::to_value` then `to_string`; `serde_json`'s default map is sorted, so the `preserve_order` feature must stay off (add a comment in `Cargo.toml`).
  - `pub fn entry_hash(e: &LedgerEntry) -> String`: lowercase hex SHA-256 of `canonical_json(e)`.
  - `pub fn chain(claims: &[Claim], prev_hash: &str) -> Vec<LedgerEntry>`: claims' `project` is copied as-is; mapping local project ids to server ids is the daemon's job before calling this.
  - `pub fn verify_chain(entries: &[LedgerEntry], prev_hash: &str) -> Result<(), ChainError>`; `pub struct ChainError { pub index: usize, pub reason: &'static str }` with reasons `"bad_prev_hash"` and `"bad_hash"`.

- [ ] **Step 1: Write the failing tests:**

```rust
#[test] fn canonical_json_is_sorted_and_compact() {
    let e = sample_entry(); // action_id "a1", ts 1000, project Some("p"), kind "prompt", verified 60, unverified 0,
                            // reasons ["capped"], features {"relevance_pm": 600}, prev_hash GENESIS_HASH
    assert_eq!(canonical_json(&e), format!(
        r#"{{"action_id":"a1","features":{{"relevance_pm":600}},"kind":"prompt","prev_hash":"{}","project":"p","reasons":["capped"],"ruleset":"2026.11-0","ts":1000,"unverified_s":0,"v":1,"verified_s":60}}"#,
        GENESIS_HASH));
}
#[test] fn chain_links_and_verifies() {
    let entries = chain(&[claim("a1"), claim("a2")], GENESIS_HASH);
    assert_eq!(entries[1].prev_hash, entries[0].hash);
    assert!(verify_chain(&entries, GENESIS_HASH).is_ok());
}
#[test] fn tampering_is_detected_at_the_right_index() {
    let mut entries = chain(&[claim("a1"), claim("a2"), claim("a3")], GENESIS_HASH);
    entries[1].verified_s += 60;
    let err = verify_chain(&entries, GENESIS_HASH).unwrap_err();
    assert_eq!((err.index, err.reason), (1, "bad_hash"));
}
```

- [ ] **Step 2:** Run `cargo test -p tally-scoring ledger`. Expected: FAIL.
- [ ] **Step 3:** Implement per the interfaces.
- [ ] **Step 4:** Run the same command. Expected: PASS.
- [ ] **Step 5:** Commit: `feat(scoring): hash-chained ledger`.

---

### Task 6: Text features (feature `text`)

**Files:**
- Create: `src/text.rs` (compiled only with `--features text`)
- Modify: `crates/tally-scoring/Cargo.toml` (`[features] text = ["dep:zstd"]`)

**Interfaces:**
- Produces:
  - `pub fn tokens(text: &str) -> Vec<String>`: lowercase, then scan for runs matching `[a-z_][a-z0-9_]{2,}` (manual scan, no regex crate), dropping words in `STOPWORDS`.
  - `pub const STOPWORDS: &[&str]`: the 40 most common English function words (the, and, for, that, this, with, you, are, not, but, have, was, can, all, any, its, from, they, will, would, there, their, what, about, which, when, make, like, just, into, than, then, them, these, some, could, other, only, also, been).
  - `pub struct Vocab` with `pub fn from_texts<'a>(texts: impl IntoIterator<Item = &'a str>) -> Vocab` (stops adding after `VOCAB_MAX` distinct tokens; the caller passes most recently modified files first) and `pub fn extend(&mut self, text: &str)` for the last agent output and last error output.
  - `pub fn relevance_pm(prompt: &str, vocab: &Vocab) -> u16`: `1000 × |prompt tokens ∩ vocab| / max(|prompt tokens|, 1)`, counting distinct prompt tokens.
  - `pub fn novelty_bytes(context_tail: &str, prompt: &str) -> u32`: zstd level 3, `len(c(tail ‖ prompt)).saturating_sub(len(c(tail)))`; `tail` = last `CONTEXT_TAIL_BYTES` (8192, add to `ruleset.rs`) bytes, cut at a char boundary.
  - `pub fn prompt_features(prompt: &str, vocab: &Vocab, context_tail: &str) -> (u16, u32)`: `(relevance, novelty)`, with relevance forced to 0 when `novelty < NOVELTY_MIN`.

- [ ] **Step 1: Write the failing tests:**

```rust
#[test] fn tokens_lowercase_and_drop_stopwords_and_short() {
    assert_eq!(tokens("The Jump_Height is 2x OK"), vec!["jump_height"]);
}
#[test] fn plain_language_prompt_matches_prose_vocab() {
    let v = Vocab::from_texts(["// jump gravity tuning", "# A tiny game"]);
    assert_eq!(relevance_pm("the jump feels floaty", &v), 333);
}
#[test] fn lorem_ipsum_is_irrelevant() {
    let v = Vocab::from_texts(["fn player_jump(gravity: f32) {}"]);
    assert_eq!(relevance_pm("lorem ipsum dolor sit amet consectetur", &v), 0);
}
#[test] fn repeating_the_context_has_no_novelty() {
    let tail = "error[E0382]: borrow of moved value: `player` at src/main.rs:42".repeat(20);
    let (rel, nov) = prompt_features(&tail[..200], &Vocab::from_texts([tail.as_str()]), &tail);
    assert!(nov < NOVELTY_MIN);
    assert_eq!(rel, 0);
}
```

- [ ] **Step 2:** Run `cargo test -p tally-scoring --features text text`. Expected: FAIL.
- [ ] **Step 3:** Implement per the interfaces.
- [ ] **Step 4:** Run the same command. Expected: PASS. Also run `cargo build -p tally-scoring --target wasm32-unknown-unknown` (default features) and confirm it still builds without zstd.
- [ ] **Step 5:** Commit: `feat(scoring): local text features`.

---

### Task 7: WebAssembly bindings and CI

**Files:**
- Create: `src/wasm.rs`, `wasm-smoke/package.json`, `wasm-smoke/smoke.test.ts`, `.github/workflows/scoring.yml`
- Modify: `crates/tally-scoring/Cargo.toml` (`crate-type = ["cdylib", "rlib"]`; `[features] wasm = ["dep:wasm-bindgen"]`)

**Interfaces:**
- Consumes: `score` (Task 4), `chain`/`verify_chain` (Task 5).
- Produces (JS names via `#[wasm_bindgen(js_name = ...)]`), the contract the server plan uses:
  - `scoreJson(eventsJson: string): string`: JSON array of `Event` in, JSON array of `Claim` out; throws a JS `Error` with the serde message on bad input.
  - `verifyChainJson(entriesJson: string, prevHash: string): number`: `-1` if valid, otherwise the failing index.
  - `rulesetVersion(): string`

- [ ] **Step 1: Write the failing smoke test** `wasm-smoke/smoke.test.ts`:

```ts
import { expect, test } from "bun:test";
import { scoreJson, verifyChainJson, rulesetVersion } from "../crates/tally-scoring/pkg/tally_scoring.js";

test("ruleset version", () => expect(rulesetVersion()).toBe("2026.11-0"));
test("lunch scenario matches native", () => {
  const ev = (id: string, ts: number, kind: object) => ({ v: 1, id, ts: ts * 1000, source: "t", project: "p", session: null, ...kind });
  const p = { kind: "prompt", chars: 200, relevance_pm: 600, novelty_bytes: 90 };
  const claims = JSON.parse(scoreJson(JSON.stringify([ev("1", 0, p), ev("2", 60, { kind: "idle", end_ts: 1_860_000 }), ev("3", 1900, p)])));
  expect(claims.map((c: any) => c.verified_s)).toEqual([120, 540]);
});
test("bad input throws", () => expect(() => scoreJson("not json")).toThrow());
test("empty chain is valid", () => expect(verifyChainJson("[]", "0".repeat(64))).toBe(-1));
```

- [ ] **Step 2:** Run `wasm-pack build crates/tally-scoring --target nodejs --features wasm && cd wasm-smoke && bun test`. Expected: FAIL (exports missing).
- [ ] **Step 3:** Implement `src/wasm.rs` with the three exports.
- [ ] **Step 4:** Run the same command. Expected: 4 tests PASS.
- [ ] **Step 5:** Write `.github/workflows/scoring.yml`: on push and pull request, a matrix job (`ubuntu-latest`, `windows-latest`, `macos-latest`) running `cargo test -p tally-scoring --all-features`, plus an Ubuntu job that installs `wasm-pack` and Bun and runs the Step 2 command.
- [ ] **Step 6:** Commit: `feat(scoring): wasm bindings and CI`.

---

## Done when

- `cargo test -p tally-scoring --all-features` passes on all three OSes in CI.
- The WASM smoke test passes and returns the same lunch-scenario numbers as the native test.
- Every constant in spec §7 appears in `ruleset.rs` with the spec's value.
