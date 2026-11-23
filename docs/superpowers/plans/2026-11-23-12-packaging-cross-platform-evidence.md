# Packaging, Cross-Platform Presence, and Evidence Harnesses Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Tally runs on macOS and Linux as well as Windows (idle detection included), installs and registers itself with one command, ships as signed per-platform binaries, and has the measurement tooling behind the pitch's "Evidence before we ask": resource numbers, a Hackatime-vs-Tally replay comparator, a red-team harness, and a cost sheet.

**Architecture:** (a) New idle probes in `crates/tally-daemon/src/presence/` behind the `IdleProbe` trait from plan 04, chosen by one `select_probe()`. (b) New daemon modules `detect.rs`, `init.rs`, `service.rs`, `lock.rs` plus `tally init` and `tally service ...` subcommands; service files are rendered by pure functions and golden-tested. (c) Release engineering in `.github/workflows/release.yml`, `install/`, `packaging/`. (d) A separate crate `crates/tally-evidence` (binary `tally-evidence`, depends only on `tally-scoring`, never shipped to users) holds the replay comparator, red-team scenarios, the resource harness and the cost sheet generator. Studies (pilot, blind grading) are human work; this plan builds the tooling only.

**Tech Stack:** Rust stable 1.85+, existing `tally-daemon`/`tally-scoring`. New daemon dependencies, each only on the target that needs it: `x11rb` (feature `screensaver`, Linux), `zbus` (Linux, blocking API), `wayland-client` + `wayland-protocols` (Linux, `staging` feature), `fs2` (all). New evidence-crate dependencies: `sysinfo`, `serde`, `serde_json`, `clap`, `anyhow`. Shell: PowerShell 5.1 and POSIX sh. CI: GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-11-22-daemon-roadmap-design.md` (section 8), `PITCH.md` (Platforms and hardware, Threat model, Privacy, Cost, Evidence before we ask), `docs/spec/scoring.md` (§2 events, idle rules), plans 03 and 04.

## Assumes from earlier plans

Plans 03 and 04 are written but not executed (and plan 04's file did not exist when this plan was written), so every name below comes from the index `README.md` (C1, C5) and from this plan's reading of the code on `main`. **Before Task 1, open the merged plan 03/04 code and reconcile each name; where the code differs, the code wins and this plan's step text gets the code's names.**

| Assumed | From | Used in |
|---|---|---|
| `Source` trait, `Poll { events, watermark }`, `SourceState`, `source_state` table (C1) | plan 03, `crates/tally-daemon/src/source.rs` | Tasks 2, 10 |
| `tally run` poll loop; `<home>/config.toml` loader with per-source tables, unknown keys an error (C5) | plan 03 | Tasks 7, 9 |
| Claude Code adapter id `claude-code`, config table `[claude_code]` | plan 03 | Task 7 |
| Presence module `crates/tally-daemon/src/presence/mod.rs` with `pub trait IdleProbe { fn name(&self) -> &str; fn idle_ms(&mut self) -> Result<u64, ProbeError>; }`, `pub enum ProbeError { Unsupported(String), Transient(String) }`, and a `PresenceSource<P: IdleProbe>` implementing `Source` (id `presence`) that turns samples into `Kind::Idle` spans when idle exceeds `IDLE_SAMPLE_MIN` (30 s) and emits the closed span on resume | plan 04 (**my guess at names**) | Tasks 2 to 6 |
| Presence coverage rule: a presence source whose probe works advances its watermark to its last good sample; one whose probe is `Unsupported` or has been failing keeps its watermark at `Some(0)`, so the horizon cannot pass it and **no claim becomes final without presence coverage** | plan 04 (**my guess; this plan relies on it and Task 2 tests it**) | Task 2 |
| Gap handling: when two samples are further apart than 3 poll intervals (laptop asleep, daemon stalled) the presence source emits an `Idle` span over the gap, never silence | plan 04 (**my guess**) | Review Focus 1, Task 6 |
| `Store`, `finalize`, `register_git_project`, `find_git_root`, `project_id`, `normalize_path`, CLI `--home`/`TALLY_HOME`, `main.rs` clap enum `Cmd` | `main` today | Tasks 7 to 10 |
| `Event`, `Kind`, `Actor`, `Claim { action_id, ts, project, kind, verified_s, unverified_s, reasons, features }`, `score()`, `ruleset::*` constants | `tally-scoring` on `main` | Tasks 15 to 18 |
| Plan 05 `tally publish` and a daily publish job inside `tally run` | plan 05 | Task 7 explainer text only (describes publishing; no code dependency) |

## Global Constraints

- No event text, ever (spec §2, C3). Nothing in this plan reads window titles, URLs, keystrokes or screen contents. Idle probes return one integer: milliseconds since the last input event, from OS APIs that do not expose what the input was. No keylogger APIs, no screen capture (PITCH Privacy).
- Honest unsupported: a platform or desktop with no working idle source reports `Unsupported` with a human-readable reason, `tally status` and `tally init` show it, and claims are not finalized (so not scored into the ledger). Never fall back to "assume present".
- One scoring implementation. `tally-evidence` calls `tally_scoring::score`; it never reimplements a rule. The Hackatime-style comparator is the only other time computation and is labelled as an approximation.
- Every OS-specific module is behind `#[cfg(target_os = ...)]` and has a pure, platform-independent core that is unit tested on every OS in CI. Code that talks to the OS is verified by the manual checklist in its task, on real hardware, and the result is recorded in `docs/platform-notes.md`.
- Never invent a vendor format or API: Tasks 1, 7 (harness locations), 8 (service managers) and 12 (signing) begin with a spike or a documented maintainer input.
- Uninstall never deletes the data directory (`<home>`: the store, ledger, config) unless the user passes `--purge`. The ledger is the user's proof; losing it silently is data loss.
- Installers and service registration are idempotent: running twice leaves one install, one service, one config.
- No secrets in the repo. Signing material comes from GitHub Actions secrets named in Task 12; the workflow degrades to unsigned artifacts (clearly named `-unsigned`) when they are absent.
- New Rust code: no warnings, `cargo clippy --all-targets -- -D warnings` clean on all three OSes. Commits follow the repo's `feat(scope): ...` style.

## Review Focus

1. **Laptop lid closed, on each OS.** Expected: while asleep no samples exist; on wake the presence source emits an `Idle` span over the gap (OS idle timers may reset or keep counting across sleep, so the code must not trust them across a gap), so sleep time is subtracted or capped, never credited as presence. The gap rule has a pure test in Task 6; a per-OS manual checklist (close lid 10 minutes, open, confirm `tally status` shows the gap as idle) is recorded for Windows, macOS, Linux/X11 and Linux/GNOME-Wayland.
2. **A Wayland compositor without idle-notify** (for example an older wlroots compositor lacking `ext-idle-notify-v1`, or GNOME with the Mutter D-Bus name absent). Expected: `select_probe()` returns `Unsupported("...")` naming the desktop, `tally status` says "presence: unsupported on this desktop, claims are not final", the presence watermark stays at 0, nothing finalizes. Tested in Task 5 with injected environment and fake probes. XWayland must not be mistaken for coverage: the XScreenSaver idle counter under Wayland only sees X clients, so Wayland sessions never use the X11 probe (Task 4 test).
3. **Two daemons for one user.** Expected: the second `tally run` exits non-zero within one second saying who holds the lock (pid) and does not touch the store. Two daemons would double-poll and race `finalize`. Different `--home` directories lock independently. Tested in Task 10. A service plus a manually started `tally run` is the realistic case.
4. **Installer run twice.** Expected: the second run with the same version replaces nothing, keeps the config, and leaves exactly one service registration; run with a newer version it stops the service, swaps the binary, restarts. Tested by Task 13's CI workflow running each installer twice on its OS.
5. **Uninstall leaves the data directory intact unless asked.** Expected: `tally service uninstall` removes the service registration only; the installers' uninstall option removes the binary and PATH entry only; `--purge` is the only path that deletes `<home>`, it prints the directory and the ledger length first and requires `--yes` when not interactive. Tests in Tasks 9 and 13.
6. **Evidence numbers that flatter.** Expected: the resource harness reports the daemon process's own RSS and CPU (the harness process excluded), records the machine description and refuses to write the published numbers file without `--machine "<description>"`. The red-team harness runs an honest control through the same scorer, so a scorer that credits nothing at all is visible (control must credit more than zero).
7. **Release artifacts.** Expected: every artifact has a SHA-256 in `SHA256SUMS`, installers verify the checksum before installing, and an unsigned build is never presented as signed.

## File Structure

```
crates/tally-daemon/
  Cargo.toml                       target-gated deps (x11rb, zbus, wayland-*), fs2
  src/presence/macos.rs            CGEventSourceSecondsSinceLastEventType probe
  src/presence/x11.rs              XScreenSaver idle probe
  src/presence/mutter.rs           org.gnome.Mutter.IdleMonitor probe (D-Bus)
  src/presence/wayland_idle.rs     ext-idle-notify-v1 probe
  src/presence/select.rs           select_probe(env) -> Box<dyn IdleProbe> | Unsupported
  src/detect.rs                    find installed harnesses (pure over a filesystem view)
  src/init.rs                      tally init: detect, write config, register repo, print explainer
  src/explainer.txt                the one-page explainer for parents (include_str!)
  src/service.rs                   render + install/start/stop/status/uninstall per OS
  src/lock.rs                      single-instance lock on <home>/tally.lock
  src/main.rs                      adds `init`, `service`, and the lock in `run`
  tests/service_golden.rs          rendered unit files against golden files
  tests/golden/                    tally.service, dev.tally.agent.plist, task.xml
crates/tally-evidence/
  Cargo.toml                       binary `tally-evidence`
  src/lib.rs
  src/hackatime.rs                 Hackatime-style hours from an event stream
  src/replay.rs                    comparator + removed-minutes timeline
  src/scenarios.rs                 red-team attack generators + honest control
  src/redteam.rs                   run scenarios through score(), report credit per attack
  src/load.rs                      realistic replay load generator for the resource harness
  src/resources.rs                 spawn `tally run`, sample RSS/CPU, write numbers file
  src/cost.rs                      cost sheet from measured ledger sizes
  src/main.rs                      clap: replay | redteam | load | resources | cost
  tests/*.rs
.github/workflows/release.yml      build matrix, checksums, signing, GitHub release
.github/workflows/installers.yml   run installers twice on three OSes
install/install.ps1, install/install.sh
packaging/winget/*.yaml, packaging/homebrew/tally.rb   (optional tasks)
docs/platform-notes.md             per-OS manual verification results
docs/evidence/                     published outputs: resources.md, redteam.md, cost.md, replay samples
docs/release.md                    maintainer inputs: accounts, secrets, how to cut a release
```

---

## Part A: Presence on macOS and Linux

### Task 1: Spike, record what each OS actually returns

**Files:**
- Create: `docs/platform-notes.md`, `crates/tally-daemon/examples/idle_spike.rs`

Nothing in later tasks may assume an API's behaviour that this task did not observe on real hardware.

**Interfaces:**
- Produces: `docs/platform-notes.md` with one section per environment: Windows 11, macOS (Apple silicon and, if available, Intel), Ubuntu/X11, Fedora/GNOME Wayland, one wlroots Wayland compositor (Sway). Each section records: the call used, units, the value while typing, after 60 s untouched, **across a 2-minute lid-close sleep** (does the counter reset, keep counting, or jump), and for Wayland whether `ext-idle-notify-v1` and `org.gnome.Mutter.IdleMonitor` are present (`wayland-info | grep idle`, `busctl --user introspect org.gnome.Mutter.IdleMonitor /org/gnome/Mutter/IdleMonitor/Core`).
- `idle_spike.rs`: prints `idle_ms` once per second using the platform's call; stub per OS, compiled only there. It is a throwaway example, not shipped.

- [ ] **Step 1:** On each available machine, run a three-line probe by hand first (macOS: `ioreg -c IOHIDSystem | awk '/HIDIdleTime/ {print $NF/1000000000}'` as an independent cross-check; X11: `xprintidle`; GNOME: `gdbus call --session --dest org.gnome.Mutter.IdleMonitor --object-path /org/gnome/Mutter/IdleMonitor/Core --method org.gnome.Mutter.IdleMonitor.GetIdletime`). Paste outputs into `docs/platform-notes.md`.
- [ ] **Step 2:** Write `idle_spike.rs` for the current OS only; run it with `cargo run -p tally-daemon --example idle_spike`; confirm it agrees with the hand probe within 1 s. Record the sleep behaviour.
- [ ] **Step 3:** For machines you do not have, write `NOT VERIFIED: <reason>` in that section; later tasks keep their manual checklist open for it. Do not guess.
- [ ] **Step 4:** Commit: `docs: record idle API behaviour per platform`.

---

### Task 2: Probe selection and the unsupported coverage rule

**Files:**
- Create: `crates/tally-daemon/src/presence/select.rs`
- Modify: `crates/tally-daemon/src/presence/mod.rs` (`pub mod select;`, and `mod macos; mod x11; mod mutter; mod wayland_idle;` each under its `cfg`)

**Interfaces:**
- Consumes: `IdleProbe`, `ProbeError`, `PresenceSource` (plan 04, see Assumes).
- Produces (`select.rs`):
  - `pub struct DesktopEnv { pub os: &'static str, pub session_type: Option<String>, pub wayland_display: Option<String>, pub x_display: Option<String>, pub desktop: Option<String> }` with `DesktopEnv::from_process()` reading `std::env::consts::OS`, `XDG_SESSION_TYPE`, `WAYLAND_DISPLAY`, `DISPLAY`, `XDG_CURRENT_DESKTOP`.
  - `#[derive(Debug, PartialEq)] pub enum Choice { Windows, MacOs, X11, MutterWayland, ExtIdleNotify, Unsupported(String) }`
  - `pub fn choose(env: &DesktopEnv, mutter_present: bool, ext_idle_present: bool) -> Choice`: pure.
  - `pub fn select_probe() -> Box<dyn IdleProbe>`: builds `DesktopEnv`, queries the two `*_present` facts through each module's `available()` function, calls `choose`, constructs the probe, or an `UnsupportedProbe(reason)` whose `idle_ms` always returns `Err(ProbeError::Unsupported(reason))`.
- Rules in `choose`: `windows` gives `Windows`; `macos` gives `MacOs`; `linux` with a Wayland session (`session_type == "wayland"` or `wayland_display` set) gives `MutterWayland` if `mutter_present`, else `ExtIdleNotify` if `ext_idle_present`, else `Unsupported("Wayland compositor <desktop> offers neither org.gnome.Mutter.IdleMonitor nor ext-idle-notify-v1")`; `linux` with an X session and `x_display` set gives `X11`; anything else gives `Unsupported`. **Wayland wins over X11 even when `DISPLAY` is also set (XWayland).**

- [ ] **Step 1: Write the failing tests** in `select.rs`:

```rust
fn env(os: &'static str, session: Option<&str>, wl: bool, x: bool) -> DesktopEnv {
    DesktopEnv { os, session_type: session.map(String::from),
        wayland_display: wl.then(|| "wayland-0".into()), x_display: x.then(|| ":0".into()),
        desktop: Some("sway".into()) }
}
#[test] fn windows_and_macos_choose_their_native_probe() {
    assert_eq!(choose(&env("windows", None, false, false), false, false), Choice::Windows);
    assert_eq!(choose(&env("macos", None, false, false), false, false), Choice::MacOs);
}
#[test] fn x11_session_chooses_x11() {
    assert_eq!(choose(&env("linux", Some("x11"), false, true), false, false), Choice::X11);
}
#[test] fn wayland_with_xwayland_never_uses_the_x11_probe() {
    assert_eq!(choose(&env("linux", Some("wayland"), true, true), false, true), Choice::ExtIdleNotify);
    assert_eq!(choose(&env("linux", Some("wayland"), true, true), true, true), Choice::MutterWayland);
}
#[test] fn wayland_without_idle_support_is_unsupported_with_a_reason() {
    match choose(&env("linux", Some("wayland"), true, true), false, false) {
        Choice::Unsupported(r) => assert!(r.contains("ext-idle-notify") && r.contains("sway")),
        c => panic!("{c:?}"),
    }
}
#[test] fn headless_linux_is_unsupported() {
    assert!(matches!(choose(&env("linux", None, false, false), false, false), Choice::Unsupported(_)));
}
#[test] fn unsupported_probe_keeps_the_presence_watermark_at_zero_so_nothing_finalizes() {
    // build PresenceSource over UnsupportedProbe; poll at a late `now`; assert
    //   poll.events is empty and poll.watermark == Some(0)
    // then Store::open_in_memory(), append a claude-code prompt + edit 10 minutes apart,
    // set_watermark("claude-code", i64::MAX / 4), set_watermark("presence", 0),
    // and assert finalize(&mut store).unwrap().appended == 0 and provisional > 0.
}
```

- [ ] **Step 2:** Run `cargo test -p tally-daemon select`. Expected: FAIL.
- [ ] **Step 3:** Implement `DesktopEnv`, `Choice`, `choose`, `UnsupportedProbe`, and `select_probe` (the `available()` functions are stubs returning `false` until Tasks 4 and 5; Windows uses plan 04's probe). If plan 04's `PresenceSource` does not already return `watermark: Some(0)` for an unsupported probe, add that behaviour there and cover it with the last test.
- [ ] **Step 4:** Run `cargo test -p tally-daemon`. Expected: PASS on all three OSes.
- [ ] **Step 5:** Make `tally status` print one line `presence: <probe name> (ok)` or `presence: unsupported: <reason> (claims are not finalized)`. Add a test of the line builder (`fn presence_line(&dyn IdleProbe) -> String`) for both cases.
- [ ] **Step 6:** Commit: `feat(daemon): choose an idle probe per desktop, honest unsupported`.

---

### Task 3: macOS idle probe

**Files:**
- Create: `crates/tally-daemon/src/presence/macos.rs` (`#[cfg(target_os = "macos")]`)
- Modify: `crates/tally-daemon/src/presence/select.rs` (wire `Choice::MacOs`)

**Interfaces:**
- `pub struct MacProbe;` implementing `IdleProbe`, name `"macos-cgevent"`. `idle_ms` calls `CGEventSourceSecondsSinceLastEventType(kCGEventSourceStateCombinedSessionState = 0, kCGAnyInputEventType = !0u32)` through a hand-written `extern "C"` block linked with `#[link(name = "ApplicationServices", kind = "framework")]` (no crate for one function). It returns seconds as `f64`; convert with `(secs * 1000.0).round()`. A negative or NaN value is `ProbeError::Transient`.
- Pure helper, tested on every OS: `pub fn seconds_to_ms(secs: f64) -> Result<u64, ProbeError>` (rejects negative, NaN, infinite; clamps above 10 years).
- Privacy note for the explainer (Task 7): this call needs no Accessibility or Input Monitoring permission because it reports only elapsed time, not events. **Verify on a real Mac in Step 3**; if macOS prompts for a permission, record it in `platform-notes.md` and stop: that changes the privacy story and the maintainer decides.

- [ ] **Step 1: Write the failing test** (runs on every OS, in `macos.rs` outside the cfg-gated block, or in a `presence/idle_math.rs` shared module):

```rust
#[test] fn seconds_convert_and_reject_nonsense() {
    assert_eq!(seconds_to_ms(1.5).unwrap(), 1500);
    assert_eq!(seconds_to_ms(0.0004).unwrap(), 0);
    assert!(seconds_to_ms(-1.0).is_err());
    assert!(seconds_to_ms(f64::NAN).is_err());
    assert_eq!(seconds_to_ms(1e18).unwrap(), 315_360_000_000); // clamp: 10 years in ms
}
```
- [ ] **Step 2:** Run `cargo test -p tally-daemon seconds_convert`. Expected: FAIL; implement; PASS.
- [ ] **Step 3:** Implement `MacProbe` (macOS only). Manual check on a real Mac: `cargo run -p tally-daemon --example idle_spike` while idle shows a growing number, touching the trackpad resets it, no permission prompt appears. Record in `docs/platform-notes.md`.
- [ ] **Step 4:** CI: `macos-latest` already builds the crate; add a `#[cfg(target_os = "macos")] #[test] fn probe_returns_a_number()` that only asserts `idle_ms().is_ok()` (a CI runner has no human, but the call must succeed).
- [ ] **Step 5:** Commit: `feat(daemon): macOS idle probe`.

---

### Task 4: Linux X11 idle probe

**Files:**
- Create: `crates/tally-daemon/src/presence/x11.rs` (`#[cfg(target_os = "linux")]`)
- Modify: `crates/tally-daemon/Cargo.toml`: `[target.'cfg(target_os = "linux")'.dependencies] x11rb = { version = "0.13", features = ["screensaver"] }`

**Interfaces:**
- `pub fn available() -> bool`: `DISPLAY` set, connection opens, and the `MIT-SCREEN-SAVER` extension is present (`x11rb::protocol::screensaver::query_version`).
- `pub struct X11Probe { conn, root }`, `X11Probe::connect() -> Result<Self, ProbeError>`; `idle_ms` sends `screensaver::query_info(root)` and returns `ms_since_user_input` as `u64`. A broken connection returns `ProbeError::Transient` and the next call reconnects once.
- The selection rule from Task 2 already ensures this probe is never used in a Wayland session. XScreenSaver is the X server's own idle counter; it exposes no input contents.

- [ ] **Step 1: Write the failing test** in `x11.rs` (Linux only, skipped when no X server): `if std::env::var("DISPLAY").is_err() { return; }`, then `assert!(X11Probe::connect().unwrap().idle_ms().is_ok())`. CI runs it under `xvfb-run -a cargo test -p tally-daemon x11` (add `sudo apt-get install -y xvfb` to the Linux job in `daemon.yml`).
- [ ] **Step 2:** Run it under xvfb. Expected: FAIL (not defined).
- [ ] **Step 3:** Implement. Make the Xvfb test also assert that `idle_ms` is at least the sleep time after `sleep(1.2s)` with no input (`>= 1000`).
- [ ] **Step 4:** Manual check on a real X11 desktop (Task 1's machine): typing resets the counter. Record in `docs/platform-notes.md`.
- [ ] **Step 5:** Wire `Choice::X11` in `select_probe`. Commit: `feat(daemon): X11 idle probe via XScreenSaver`.

---

### Task 5: Wayland idle probes (Mutter and ext-idle-notify)

**Files:**
- Create: `crates/tally-daemon/src/presence/mutter.rs`, `crates/tally-daemon/src/presence/wayland_idle.rs` (both Linux only)
- Modify: `crates/tally-daemon/Cargo.toml` (Linux target deps `zbus = { version = "4", default-features = false, features = ["blocking-api", "async-io"] }`, `wayland-client = "0.31"`, `wayland-protocols = { version = "0.32", features = ["client", "staging"] }`), `select.rs`

**Interfaces:**
- `mutter::available() -> bool`: the session bus has the name `org.gnome.Mutter.IdleMonitor`. `MutterProbe::idle_ms` calls `GetIdletime` on `/org/gnome/Mutter/IdleMonitor/Core` (returns `t` milliseconds). Name `"mutter-idlemonitor"`.
- `wayland_idle::available() -> bool`: connect to `WAYLAND_DISPLAY`, list globals, true iff `ext_idle_notifier_v1` is advertised. `ExtIdleProbe` name `"ext-idle-notify"`.
- `ext-idle-notify-v1` does not report elapsed idle time; it reports `idled` after a timeout and `resumed` on input. `ExtIdleProbe` therefore runs one background thread owning the Wayland event queue with a notification at timeout `IDLE_SAMPLE_MIN` (30 000 ms), and keeps an `IdleTracker` behind a mutex. While idle, `idle_ms` returns `timeout_ms + elapsed since the idled event`; after `resumed` it returns `0`. Below the timeout the true idle time is unknown and reported as `0`. Document in the module that resolution below 30 s is not available on this path; the scorer only uses idle spans over 30 s (`IDLE_SAMPLE_MIN`), so no precision is lost for scoring.
- Pure core (tested everywhere): `pub struct IdleTracker { timeout_ms: u64, idle_since: Option<i64> }` with `on_idled(now_ms)`, `on_resumed()`, `idle_ms(now_ms) -> u64`.

- [ ] **Step 1: Write the failing tests** for the pure core, in `wayland_idle.rs` outside the cfg-gated parts (so they run on every OS, move `IdleTracker` into `presence/idle_math.rs` if cfg gating makes that awkward):

```rust
#[test] fn tracker_reports_zero_until_idled_then_timeout_plus_elapsed() {
    let mut t = IdleTracker::new(30_000);
    assert_eq!(t.idle_ms(1_000), 0);
    t.on_idled(100_000);                       // compositor says: idle for 30 s as of now
    assert_eq!(t.idle_ms(100_000), 30_000);
    assert_eq!(t.idle_ms(160_000), 90_000);
    t.on_resumed();
    assert_eq!(t.idle_ms(170_000), 0);
}
```
- [ ] **Step 2:** Run `cargo test -p tally-daemon tracker`. Expected: FAIL; implement `IdleTracker`; PASS.
- [ ] **Step 3:** Implement `MutterProbe` (blocking zbus proxy; a missing name makes `available()` false, a call error is `Transient`).
- [ ] **Step 4:** Implement `ExtIdleProbe` with the background thread; thread death makes `idle_ms` return `Transient` so the presence source stops advancing its watermark (no claims final while blind).
- [ ] **Step 5:** Wire both into `select_probe`. Add the compile-only CI check `cargo check -p tally-daemon --target x86_64-unknown-linux-gnu` already covered by the Linux job.
- [ ] **Step 6:** Manual checks, recorded in `platform-notes.md`: GNOME Wayland (Mutter path: idle grows, input resets); Sway or another wlroots compositor with `ext-idle-notify-v1` (ExtIdle path); and a compositor or environment **without** either (for example an old wlroots build, or `WAYLAND_DISPLAY` pointing at a bare `weston` without the protocol): `tally status` must print the unsupported line from Task 2. Write `NOT VERIFIED` for any not available.
- [ ] **Step 7:** Commit: `feat(daemon): Wayland idle probes (Mutter, ext-idle-notify)`.

---

### Task 6: Sleep and lid-close gap handling, verified per OS

**Files:**
- Modify: `crates/tally-daemon/src/presence/mod.rs` (only if plan 04's gap rule is missing or differs), `docs/platform-notes.md`

**Interfaces:**
- Rule (Review Focus 1): if `now - last_good_sample_ts > 3 * poll_interval_ms`, `PresenceSource` emits `Kind::Idle { end_ts: now }` starting at `last_good_sample_ts`, id `presence:<start>:idle`, before any new sample logic, and the probe's value right after the gap is ignored for that one sample (a probe that resets to 0 on wake would otherwise hide the gap; one that keeps counting would double count). The watermark stays below the gap's start until the span is emitted.

- [ ] **Step 1: Write the failing test** (fake probe, fake clock, no OS): samples at 0 s, 5 s, 10 s with the probe returning 0; then the clock jumps to 610 s (lid closed 10 minutes) and the probe returns 0 on wake. Assert exactly one `Idle` event with `ts == 10_000` and `end_ts == 610_000`, and that scoring `[prompt at 5 s, prompt at 615 s]` credits no time for the gap (`score(...)` second claim `verified_s` is at most the first-action allowance, and reasons contain `idle_subtracted`).
- [ ] **Step 2:** Run. Expected: FAIL or PASS-already (if plan 04 implemented it; then keep the test and skip Step 3).
- [ ] **Step 3:** Implement the rule; run; PASS.
- [ ] **Step 4:** Manual lid-close checklist on every machine from Task 1: run `tally run` with a debug log, work for a minute, close the lid for 10 minutes, open it, type one prompt. Then `tally status` must show the 10 minutes as idle and the prompt's claim must not include them. Record pass or `NOT VERIFIED` per OS in `docs/platform-notes.md`. Also test hibernate on Windows and suspend-to-RAM on Linux if available.
- [ ] **Step 5:** Commit: `test(daemon): sleep gap becomes an idle span on every OS`.

---

## Part B: Harness detection, `tally init`, background service

### Task 7: Harness detection, `tally init`, and the explainer

**Files:**
- Create: `crates/tally-daemon/src/detect.rs`, `crates/tally-daemon/src/init.rs`, `crates/tally-daemon/src/explainer.txt`
- Modify: `crates/tally-daemon/src/lib.rs`, `src/main.rs` (`Init` subcommand)

**Spike first (Step 1).** Where each harness keeps its session logs is vendor knowledge that changes. Do not trust this plan's candidate list; confirm on a machine that has the tool.

**Interfaces:**
- `detect.rs`:
  - `pub struct Found { pub id: &'static str, pub path: PathBuf, pub adapter_ready: bool }` where `adapter_ready` is true only for harnesses whose adapter exists in this build (today `claude-code`; plan 08 adds more).
  - `pub trait Fs { fn is_dir(&self, p: &Path) -> bool; fn home(&self) -> Option<PathBuf>; fn env(&self, k: &str) -> Option<String>; }` with a real impl and a test fake.
  - `pub fn detect_harnesses(fs: &dyn Fs) -> Vec<Found>`: table-driven (`const CANDIDATES: &[(&str, &[&str])]` of id and home-relative directories). Only checks that a directory exists; it never reads inside it.
- `init.rs`:
  - `pub struct InitPlan { pub home: PathBuf, pub found: Vec<Found>, pub repo: Option<PathBuf>, pub presence: String, pub config_toml: String }` and `pub fn plan_init(fs, cwd, presence_line) -> InitPlan` (pure).
  - `pub fn apply_init(plan: &InitPlan, store: &mut Store, write_config: bool) -> Result<InitReport>`: creates `<home>`, writes `<home>/config.toml` **only if it does not exist** (an existing config is never overwritten; report "kept existing config"), registers the current git repo with `register_git_project` (no public name unless `--name`), returns lines to print.
  - `pub const EXPLAINER: &str = include_str!("explainer.txt");` printed by `tally init` before it asks to continue, and by `tally init --explain` alone.
- CLI: `tally init [--yes] [--name <public-name>] [--no-service] [--explain]`. Interactive default asks `Continue? [y/N]` after printing the explainer and the detection summary; `--yes` skips the prompt. With `--yes` and without `--no-service` it ends by running `tally service install` (Task 9).
- `explainer.txt` (one printed page, plain text, 70 columns, written for a parent; the pitch promises it ships with the installer). Required content, each as its own short paragraph: what Tally is for; **what is captured** (when you send a prompt, when you approve or interrupt the agent, when you run tests, when you commit, how long the keyboard and mouse were idle; counts and lengths, never the words); **what is never captured** (prompt text, file contents, commands, window titles, web addresses, keystrokes, screenshots); what leaves the computer (nothing unless the user publishes; then only a ledger of project name, action type, seconds and a reason code); how to see the data (`tally status`), how to stop it (`tally service stop`), how to remove it (`tally service uninstall`, and `--purge` to delete all data); who to ask. Every claim in it must match the code; Step 6 below checks the two lists mechanically.

- [ ] **Step 1 (spike):** On a machine with Claude Code, and any other harness you can, list the log directories and record them in `docs/platform-notes.md` ("Harness log locations", per OS, with the tool version). Put only verified entries in `CANDIDATES`. Claude Code's `~/.claude/projects` is known from the roadmap design; others are `NOT VERIFIED` until seen, and are omitted from `CANDIDATES` until plan 08 adds their adapters.
- [ ] **Step 2: Write the failing tests** (`detect.rs`):

```rust
#[test] fn finds_claude_code_when_its_directory_exists() {
    let fs = FakeFs::with_dirs(&["/home/o/.claude/projects"]);
    let f = detect_harnesses(&fs);
    assert_eq!(f.len(), 1);
    assert_eq!((f[0].id, f[0].adapter_ready), ("claude-code", true));
}
#[test] fn nothing_found_is_an_empty_list_not_an_error() {
    assert!(detect_harnesses(&FakeFs::with_dirs(&[])).is_empty());
}
```
- [ ] **Step 3:** Run `cargo test -p tally-daemon detect`; FAIL; implement `Fs`, `CANDIDATES`, `detect_harnesses`; PASS.
- [ ] **Step 4: Write the failing tests** (`init.rs`, using `Store::open_in_memory` and a temp dir with a `.git` directory):

```rust
#[test] fn init_registers_the_repo_and_writes_config_once() {
    // plan_init with cwd = <tmp>/repo/src (repo has .git); apply_init twice.
    // assert: project registered with id == project_id(<tmp>/repo)
    // assert: config.toml exists after the first apply, and after the second apply its bytes are unchanged
    //         even though the test edited it in between (kept existing config)
}
#[test] fn init_outside_a_repo_still_succeeds_and_says_so() {
    // cwd without .git: repo is None; report contains "no git repository here; run `tally project add <path>`"
}
#[test] fn config_toml_only_contains_keys_the_loader_accepts() {
    // parse plan_init(...).config_toml with the plan-03 config loader; must not error on unknown keys
}
#[test] fn explainer_lists_what_is_never_captured() {
    for w in ["prompt text", "file contents", "window titles", "web addresses", "keystrokes", "screenshots"] {
        assert!(EXPLAINER.contains(w), "{w}");
    }
    assert!(EXPLAINER.lines().all(|l| l.len() <= 72));
}
```
- [ ] **Step 5:** Run; FAIL; implement `plan_init`, `apply_init`, the CLI wiring, and `explainer.txt`; PASS. `config_toml` is written with the key names that plan 03's loader defines (read its loader; do not guess). It enables each found, adapter-ready harness and nothing else.
- [ ] **Step 6:** Add the test `explainer_matches_event_kinds`: for every `Kind` variant name in `tally_scoring::event` (`prompt`, `interrupt`, ...) the explainer's captured-list is reviewed by the engineer once and the test pins the list of kinds (`const KINDS_DESCRIBED: [&str; 9]`) so adding a new event kind fails this test until the explainer is revisited.
- [ ] **Step 7:** Manual: run `tally init --home <tmp>` in this repo. Expected: explainer, "found: claude-code", "registered: <path>", "presence: ... (ok)" or unsupported line, config written. Run again: "kept existing config", no second project row.
- [ ] **Step 8:** Commit: `feat(daemon): harness detection, tally init, and the parents' explainer`.

---

### Task 8: Service definitions, rendered and golden-tested

**Files:**
- Create: `crates/tally-daemon/src/service.rs` (render and command planning only in this task), `crates/tally-daemon/tests/service_golden.rs`, `crates/tally-daemon/tests/golden/tally.service`, `.../dev.tally.agent.plist`, `.../task.xml`
- Modify: `src/lib.rs`

**Spike first.** Run each OS's registration by hand once (Step 1) and keep the exact working commands in `docs/platform-notes.md`; the goldens are copied from files that worked.

**Why a user-session task and not a Windows Service.** Idle time is per interactive session (`GetLastInputInfo`, XScreenSaver, `CGEventSource`), and a true Windows Service runs in session 0 and cannot see the user's input. A per-user logon task (Windows), launchd agent (macOS) and systemd user unit (Linux) all run inside the user's session. This is a decision for the maintainer to confirm (see Decisions).

**Interfaces:**
- `pub struct ServiceSpec { pub exe: PathBuf, pub home: PathBuf, pub label: &'static str /* "dev.tally.agent" */ }`
- `pub fn render_systemd_unit(s: &ServiceSpec) -> String`: `[Unit] Description=Tally time tracker`, `After=graphical-session.target`, `PartOf=graphical-session.target`; `[Service] ExecStart=<exe> run --home <home>`, `Restart=on-failure`, `RestartSec=10`, `Environment=TALLY_HOME=<home>`; `[Install] WantedBy=graphical-session.target`. Paths with spaces are quoted per systemd rules.
- `pub fn render_launchd_plist(s) -> String`: `Label`, `ProgramArguments` (`exe`, `run`, `--home`, `home`), `RunAtLoad` true, `KeepAlive` `{ SuccessfulExit: false }`, `ProcessType` `Background`, `StandardErrorPath` `<home>/tally.log`; XML-escaped.
- `pub fn render_task_xml(s) -> String` (Windows Task Scheduler 1.2 XML, UTF-16 when written): `LogonTrigger` for the current user, `Principal` `InteractiveToken`/`LeastPrivilege`, `Settings`: `DisallowStartIfOnBatteries` false, `StopIfGoingOnBatteries` false, `ExecutionTimeLimit` `PT0S`, `RestartOnFailure` interval `PT1M` count 3, `MultipleInstancesPolicy` `IgnoreNew`, `Hidden` true.
- `pub enum Action { Install, Start, Stop, Status, Uninstall }`, `pub struct Cmd { pub program: String, pub args: Vec<String> }`, and `pub fn plan(os: &str, action: Action, s: &ServiceSpec) -> Vec<Step>` where `Step` is `WriteFile { path, contents }`, `Run(Cmd)`, or `RemoveFile(path)`. Pure. Windows: `schtasks /Create /TN Tally /XML <file> /F`, `/Run`, `/End`, `/Query`, `/Delete /F`. macOS: `launchctl bootstrap gui/<uid> <plist>`, `kickstart`, `bootout`, `print`. Linux: `systemctl --user daemon-reload`, `enable --now tally.service`, `stop`, `is-active`, `disable --now`. `Uninstall` plans **never** contain a step touching `home` (asserted below).

- [ ] **Step 1 (spike):** On each available OS, register a dummy `sleep`-style task by hand using the commands above; confirm start at login, stop, and removal. Fix the command lists in this plan's interfaces to what worked. `NOT VERIFIED` OSes are noted.
- [ ] **Step 2: Write the failing tests** (`tests/service_golden.rs`):

```rust
fn spec() -> ServiceSpec { ServiceSpec { exe: "/usr/local/bin/tally".into(), home: "/home/o/.local/share/tally".into(), label: "dev.tally.agent" } }
#[test] fn systemd_unit_matches_golden() { assert_eq!(render_systemd_unit(&spec()), include_str!("golden/tally.service")); }
#[test] fn plist_matches_golden() { assert_eq!(render_launchd_plist(&spec()), include_str!("golden/dev.tally.agent.plist")); }
#[test] fn task_xml_matches_golden() { assert_eq!(render_task_xml(&spec()), include_str!("golden/task.xml")); }
#[test] fn paths_with_spaces_and_xml_characters_are_escaped() {
    let mut s = spec(); s.exe = r"C:\Program Files\Tally & Co\tally.exe".into();
    assert!(render_task_xml(&s).contains("Tally &amp; Co"));
    assert!(render_systemd_unit(&{ let mut t = spec(); t.exe = "/opt/my tally/tally".into(); t }).contains("\"/opt/my tally/tally\""));
}
#[test] fn uninstall_never_touches_the_data_directory() {
    for os in ["windows", "macos", "linux"] {
        for step in plan(os, Action::Uninstall, &spec()) {
            if let Step::RemoveFile(p) = &step { assert!(!p.starts_with(&spec().home), "{p:?}"); }
            if let Step::Run(c) = &step { assert!(!c.args.iter().any(|a| a == spec().home.to_str().unwrap()), "{c:?}"); }
        }
    }
}
#[test] fn install_twice_plans_the_same_idempotent_steps() {
    assert_eq!(plan("linux", Action::Install, &spec()), plan("linux", Action::Install, &spec()));
}
```
- [ ] **Step 3:** Run; FAIL; write the render functions and `plan`; generate each golden by running the function once, then **review each golden by hand against the OS tool** (Step 1 evidence) before committing. Run; PASS.
- [ ] **Step 4:** Commit: `feat(daemon): render service definitions for Task Scheduler, launchd, systemd`.

---

### Task 9: `tally service` subcommands

**Files:**
- Modify: `crates/tally-daemon/src/service.rs` (executor), `src/main.rs` (`Service { install | start | stop | status | uninstall [--purge --yes] }`)
- Test: `crates/tally-daemon/tests/service_cli.rs`

**Interfaces:**
- `pub trait Runner { fn run(&mut self, c: &Cmd) -> io::Result<Output>; fn write(&mut self, p: &Path, contents: &str) -> io::Result<()>; fn remove(&mut self, p: &Path) -> io::Result<()>; }` with a real impl and a recording fake. `pub fn execute(steps: &[Step], r: &mut dyn Runner) -> Result<Vec<String>>`: `RemoveFile` and `Run` for an absent service are not errors on uninstall (idempotent), everything else stops on the first failure with the failing command and its stderr in the message.
- `status` prints one of `running (pid N)`, `installed, not running`, `not installed`, plus the presence line from Task 2 and the ledger length, and exits 0 for all three (it is a report, not a check; scripts use `--quiet` exit codes: 0 running, 3 stopped, 4 not installed).
- `uninstall --purge`: prints `<home>` and the ledger length, then requires `--yes` unless stdin is a terminal and the user types `delete`. It first runs the normal uninstall plan, then removes `<home>` last. Without `--purge`, uninstall prints `data kept in <home>`.
- `install` resolves `exe` with `std::env::current_exe()` canonicalized (a service pointing at a temporary build path is refused: if the path contains `target/debug` or `target/release` print a warning and require `--force`).

- [ ] **Step 1: Write the failing tests** (fake `Runner`, temp `home`):

```rust
#[test] fn uninstall_keeps_the_data_directory() {
    // create home with tally.db; run execute(plan(Uninstall)); assert home/tally.db still exists
}
#[test] fn purge_without_yes_noninteractive_refuses_and_deletes_nothing() { /* exit != 0, db present */ }
#[test] fn purge_with_yes_removes_home_after_the_service() {
    // assert order in the fake runner's log: service removal commands precede the home removal
}
#[test] fn uninstall_when_not_installed_succeeds() { /* fake runner returns "not found" failure for delete; execute is Ok */ }
#[test] fn install_twice_is_ok_and_leaves_one_definition() { /* WriteFile twice to the same path, Run create with /F or enable --now */ }
#[test] fn status_exit_codes_with_quiet() { /* running 0, stopped 3, not installed 4 */ }
```
- [ ] **Step 2:** Run; FAIL; implement `Runner`, `execute`, CLI; PASS.
- [ ] **Step 3:** Manual, per OS (record in `platform-notes.md`): `tally service install` then log out and in; `tally service status` says running; `tally service stop`; `tally service uninstall`; confirm `<home>/tally.db` is still there. Windows additionally: confirm no console window appears at logon and the task survives battery mode.
- [ ] **Step 4:** Commit: `feat(daemon): tally service install/start/stop/status/uninstall`.

---

### Task 10: Single-instance lock

**Files:**
- Create: `crates/tally-daemon/src/lock.rs`
- Modify: `crates/tally-daemon/Cargo.toml` (`fs2 = "0.4"`), `src/main.rs` (take the lock at the start of `run`; `finalize`, `status`, `ingest` and `verify` do not take it, they must work while the daemon runs)

**Interfaces:**
- `pub struct InstanceLock { file: File }`, `pub enum LockError { Held { pid: Option<u32> }, Io(io::Error) }`, `InstanceLock::acquire(home: &Path) -> Result<InstanceLock, LockError>`: opens `<home>/tally.lock`, `try_lock_exclusive`, writes the current pid after taking it. Dropping releases it (and a crashed process releases it, which is why this is an OS lock and not a pid file).

- [ ] **Step 1: Write the failing tests:**

```rust
#[test] fn second_acquire_on_the_same_home_fails_with_the_holders_pid() {
    let d = tempfile::tempdir().unwrap();
    let _a = InstanceLock::acquire(d.path()).unwrap();
    match InstanceLock::acquire(d.path()) {
        Err(LockError::Held { pid }) => assert_eq!(pid, Some(std::process::id())),
        r => panic!("{:?}", r.err()),
    }
}
#[test] fn released_on_drop_and_independent_per_home() {
    let (d1, d2) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    { let _a = InstanceLock::acquire(d1.path()).unwrap(); let _b = InstanceLock::acquire(d2.path()).unwrap(); }
    assert!(InstanceLock::acquire(d1.path()).is_ok());
}
```
- [ ] **Step 2:** Run; FAIL; implement; PASS on all three OSes in CI.
- [ ] **Step 3:** End-to-end test in `tests/cli.rs`: spawn `tally run --home <tmp>` (plan 03's command) as a child, wait until `<home>/tally.lock` is held, spawn a second `tally run` with the same home; assert it exits non-zero in under 2 s with stderr containing `already running` and the first child's pid; kill the first child, run again, assert it starts. Kill the child in a drop guard so a failing test does not leak a daemon.
- [ ] **Step 4:** Commit: `feat(daemon): one daemon per data directory`.

---

## Part C: Release engineering

### Task 11: Release workflow: one binary per platform, checksums

**Files:**
- Create: `.github/workflows/release.yml`
- Modify: `crates/tally-daemon/src/main.rs` (clap `version` attribute so `tally --version` prints `tally 0.1.0`), workspace root `Cargo.toml` (`lto = "thin"`, `codegen-units = 1`, `strip = true`, `panic = "abort"`)

**Targets** (decision, maintainer to confirm): `x86_64-pc-windows-msvc`, `aarch64-pc-windows-msvc`, `x86_64-apple-darwin`, `aarch64-apple-darwin`, `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`. musl gives one static Linux binary that runs on old distributions; confirm in Step 1 that every dependency (rusqlite bundled, x11rb, zbus, wayland-client with its pure-Rust backend) builds for musl.

**Interfaces:**
- Trigger: push of a tag `v*`, plus `workflow_dispatch` for a dry run that uploads artifacts and publishes nothing.
- Artifact names: `tally-<version>-<target>.zip` (Windows) or `.tar.gz` (others), each containing the single binary `tally` or `tally.exe`, `LICENSE`. A final job downloads all artifacts, writes `SHA256SUMS` (`sha256sum`-format, lines `<hex>  <file>`), and attaches everything to a draft GitHub release (`gh release create --draft`). Draft, so the maintainer reads it before publishing.
- Version check job: fails if the tag (`v0.1.0`) does not equal the daemon crate version.

- [ ] **Step 1 (spike):** Locally or in a scratch branch, run `cargo build --release -p tally-daemon --target x86_64-unknown-linux-musl` (and the aarch64 one with `cargo zigbuild`). Record in `docs/release.md` any dependency that fails and the fix. Do not write the workflow before the targets build.
- [ ] **Step 2:** Write `release.yml`: a `build` matrix job per target (Windows and macOS on native runners; Linux musl via `cargo-zigbuild` on `ubuntu-latest`), each running `cargo test -p tally-daemon` natively where the runner matches the target, then `cargo build --release -p tally-daemon --target <t>`, packaging, and `actions/upload-artifact`. Then the `checksums` job and the `release` job.
- [ ] **Step 3:** Add a test script `scripts/check-release-artifacts.sh` run in the `release` job: every file in the artifact set has a line in `SHA256SUMS`; `sha256sum -c SHA256SUMS` passes; the list of files equals the expected 6 archives. A missing target fails the release.
- [ ] **Step 4:** Dry run via `workflow_dispatch` on this branch. Expected: six artifacts and `SHA256SUMS` attached to the run (not to a release). Download the Linux x86_64 one and run `./tally --version`.
- [ ] **Step 5:** Commit: `ci: release workflow building six targets with checksums`.

---

### Task 12: Code signing and notarization, and what the maintainer must provide

**Files:**
- Create: `docs/release.md`
- Modify: `.github/workflows/release.yml`

**Nothing is invented here: this task documents inputs only the maintainer has, and wires the workflow to use them when present.**

**Interfaces, `docs/release.md` sections:**
1. **Accounts and cost.** Windows Authenticode signing certificate (a code-signing certificate from a certificate authority, or a cloud signing service) and an Apple Developer Program membership (paid, annual). PITCH budgets about $20 per month for both; the maintainer confirms current prices.
2. **GitHub Actions secrets (names this workflow reads).** Windows: `WINDOWS_CERT_PFX_BASE64` (the .pfx file, base64), `WINDOWS_CERT_PASSWORD`. macOS: `APPLE_CERT_P12_BASE64` (a "Developer ID Application" certificate with its private key, exported as .p12, base64), `APPLE_CERT_PASSWORD`, `APPLE_SIGNING_IDENTITY` (the identity string shown by `security find-identity -v -p codesigning`), `APPLE_ID`, `APPLE_TEAM_ID`, `APPLE_APP_SPECIFIC_PASSWORD` (for `notarytool`). If the maintainer chooses a cloud signing service for Windows instead of a PFX, the Windows step in Step 2 changes; record the choice in this file.
3. **What unsigned means for users**: Windows SmartScreen warning ("unrecognized app"); macOS Gatekeeper blocks a downloaded unsigned binary opened from Finder. The `curl | sh` installer (Task 13) does not set the quarantine flag, so it works either way; say this in the doc.
4. **How to cut a release**: bump version, tag `vX.Y.Z`, wait for the draft release, read the checksums, publish.

- [ ] **Step 1:** Write `docs/release.md` with the four sections. Mark every price and vendor-procedure statement `maintainer to verify`.
- [ ] **Step 2:** In `release.yml` add, per platform and only `if: ${{ secrets.X != '' }}` (use an env-to-if indirection, since secrets cannot be read in `if` directly): Windows `signtool sign /f cert.pfx /p ... /tr <timestamp-server> /td sha256 /fd sha256 tally.exe` then `signtool verify /pa`; macOS `codesign --force --options runtime --timestamp --sign "$APPLE_SIGNING_IDENTITY" tally`, zip, `xcrun notarytool submit --wait`, `codesign --verify --strict`. A bare binary cannot be stapled; note that Gatekeeper then checks notarization online on first launch.
- [ ] **Step 3:** When secrets are absent the matching artifacts are named `tally-<version>-<target>-unsigned.<ext>` and the release body starts with `UNSIGNED BUILD`. Add that check to `check-release-artifacts.sh`: a name without `-unsigned` must come from a job whose signing step ran (the job writes a `SIGNED` marker file next to the archive; the checker requires it).
- [ ] **Step 4:** Test the unsigned path with a `workflow_dispatch` run and no secrets. Expected: `-unsigned` names, `UNSIGNED BUILD` banner. The signed path **cannot be tested until the maintainer provides secrets**; say so in the PR description and in `docs/release.md`.
- [ ] **Step 5:** Commit: `ci: sign and notarize when secrets are present; document release inputs`.

---

### Task 13: One-command installers

**Files:**
- Create: `install/install.ps1`, `install/install.sh`, `.github/workflows/installers.yml`

**Interfaces:**
- Both scripts: download the archive for the machine's OS and architecture from the GitHub release (base URL overridable with `TALLY_RELEASE_BASE`, used by CI and by offline installs; version with `TALLY_VERSION`, default latest), download `SHA256SUMS`, **verify the archive's checksum before extracting and abort on mismatch**, install the binary, and print `Next: run "tally init"`. They do not register the service or start tracking: that needs the explainer and consent (`tally init`).
- `install.sh`: POSIX sh. Detects `uname -s`/`uname -m` (`Darwin`/`Linux`; `arm64`/`aarch64`/`x86_64`), unsupported combinations exit with a message. Installs to `${TALLY_INSTALL_DIR:-$HOME/.local/bin}`. If `tally --version` already equals the target version it prints `already installed` and exits 0 changing nothing. Tells the user to add the directory to `PATH` if it is not on it (does not edit shell profiles). `--uninstall` removes the binary only and prints `data kept in <home>; to delete it run: tally service uninstall --purge` (run before removing the binary, if the user wants that, so the script prints that advice first).
- `install.ps1`: PowerShell 5.1 compatible. Detects `$env:PROCESSOR_ARCHITECTURE` (`AMD64`, `ARM64`), installs to `$env:LOCALAPPDATA\Programs\Tally`, adds that directory to the **user** PATH only if absent (`[Environment]::GetEnvironmentVariable('Path','User')` check, so a second run does not duplicate it), `Get-FileHash -Algorithm SHA256`, `-Uninstall` switch with the same data-kept message. Stops a running `tally.exe` service task before replacing the binary (`schtasks /End /TN Tally`, ignoring "not found"), restarts it after if it was running.

- [ ] **Step 1: Write the CI test first** in `installers.yml` (jobs `linux`, `macos`, `windows`): build `tally` with `cargo build --release -p tally-daemon`, create a fake release directory (`tally-<v>-<target>.tar.gz|.zip` and a correct `SHA256SUMS`), then run the installer with `TALLY_RELEASE_BASE=file://<dir>` and `TALLY_INSTALL_DIR=<tmp>`. Assertions as shell steps: (a) first run exits 0 and `<tmp>/tally --version` works; (b) second run exits 0 and prints `already installed` and the binary's modification time is unchanged; (c) corrupt the checksum file, run on a clean directory: exits non-zero and installs nothing; (d) `--uninstall` removes the binary and leaves a pre-created `<tmp>/home/tally.db` untouched; (e) Windows only: after two runs the user PATH contains the install directory exactly once.
- [ ] **Step 2:** Run the workflow on the branch. Expected: FAIL (scripts do not exist).
- [ ] **Step 3:** Write both scripts to the interfaces; push; expected: all three jobs PASS.
- [ ] **Step 4:** Manual: on a clean VM or user account per OS run the documented one-liner against a real draft release (`irm https://.../install.ps1 | iex` and `curl -fsSL https://.../install.sh | sh`), then `tally init`. Record in `docs/platform-notes.md`.
- [ ] **Step 5:** Document both one-liners in `README.md` only after Task 11's first real release exists (the URL must resolve); until then leave README untouched.
- [ ] **Step 6:** Commit: `feat(install): one-command installers for Windows, macOS and Linux`.

---

### Task 14 (optional, last): winget and Homebrew manifests

**Files:**
- Create: `packaging/winget/Tally.Tally.yaml` (version, installer and locale manifests), `packaging/homebrew/tally.rb`, `scripts/render-manifests.sh`

Skip this task unless the maintainer asks. A winget submission is a pull request to `microsoft/winget-pkgs` and a Homebrew tap is a separate repository (`<owner>/homebrew-tally`); both are the maintainer's accounts and not tested here beyond linting.

**Interfaces:**
- `render-manifests.sh <version> <path-to-SHA256SUMS>` fills the version, URLs and hashes from the release's `SHA256SUMS`, so no hash is ever typed by hand.
- winget: `InstallerType: zip` with a `NestedInstallerType: portable` entry for the x64 and arm64 archives. Homebrew: one `on_macos`/`on_linux` block per arch with `url` and `sha256`, `def install; bin.install "tally"; end`, `test do; assert_match version.to_s, shell_output("#{bin}/tally --version"); end`, and a caveat saying `Run "tally init" next.`

- [ ] **Step 1: Write the failing check:** `scripts/render-manifests.sh 0.1.0 fixtures/SHA256SUMS` against a fixture checksum file; assert the output contains each of the fixture's hashes exactly once and no `TODO` or empty `sha256`.
- [ ] **Step 2:** Write the templates and script; check passes.
- [ ] **Step 3:** Lint: `winget validate packaging/winget` (Windows runner) and `brew audit --strict --formula packaging/homebrew/tally.rb` (macOS runner) in `installers.yml`, allowed to be `continue-on-error` until the first real release exists.
- [ ] **Step 4:** Commit: `feat(packaging): winget and Homebrew manifests rendered from release checksums`.

---

## Part D: Evidence harnesses (`crates/tally-evidence`)

The studies (pilot, blind grading, reviewer verdicts) are human work. This part builds the tools that produce the published numbers. Everything lands in `docs/evidence/`.

### Task 15: Evidence crate and the Hackatime-vs-Tally replay comparator

**Files:**
- Create: `crates/tally-evidence/Cargo.toml` (`[[bin]] name = "tally-evidence"`; deps `tally-scoring` path, `serde`, `serde_json`, `clap` (derive), `anyhow`), `src/lib.rs`, `src/hackatime.rs`, `src/replay.rs`, `src/main.rs`, `tests/replay.rs`
- Modify: root `Cargo.toml` (workspace member), `.github/workflows/daemon.yml` (add `cargo test -p tally-evidence`)

**Interfaces:**
- `hackatime.rs`:
  - `pub const HACKATIME_TIMEOUT_MS: i64 = 120_000;` (Hackatime/WakaTime merge heartbeats closer than the timeout; **the maintainer must confirm the timeout Hackatime really uses and replace this constant**).
  - `pub fn heartbeats_from_events(ev: &[Event]) -> Vec<i64>`: every event except `Idle` is a heartbeat at its `ts`; an `AgentBusy` span also yields a heartbeat every 30 s across `[ts, end_ts]` (an agent writing files produces editor heartbeats; PITCH: "Hackatime counts how active your AI agent is").
  - `pub fn hackatime_spans(hb: &[i64], timeout_ms: i64) -> Vec<(i64, i64)>`: sorted, deduplicated heartbeats; consecutive heartbeats closer than `timeout_ms` join one span. Total counted time is the sum of span lengths.
  - `pub fn from_heartbeat_file(text: &str) -> Result<Vec<i64>>`: NDJSON lines `{"ts": <ms>}`, for real Hackatime exports when available (preferred over the synthetic derivation; the report says which was used).
- `replay.rs`:
  - `pub struct Removed { pub minute_ts: i64, pub reason: &'static str }`, `pub struct Report { pub hackatime_s: i64, pub tally_verified_s: i64, pub tally_unverified_s: i64, pub removed: Vec<Removed>, pub added_minutes: usize, pub source_of_hackatime: &'static str }`.
  - `pub fn compare(events: &[Event], heartbeats: Option<Vec<i64>>) -> Report`.
  - `pub fn render_markdown(r: &Report, title: &str) -> String`, `pub fn render_json(r: &Report) -> String`.
- Per-minute reasons for a minute Hackatime counted and Tally did not credit, first match wins: `away` (overlaps an `Idle` span), `agent_only` (overlaps an `AgentBusy` span), `capped` (inside a claim interval whose claim has reason `capped`), `no_human_action` (anything else). "Tally credited" means the minute lies in the last `verified_s` seconds ending at a claim's `ts` (an approximation, since claims do not carry their interval; the report footnote says so). Minutes Tally credited and Hackatime did not are counted in `added_minutes` (the "five-project afternoon" case).
- CLI: `tally-evidence replay <events.ndjson> [--heartbeats <file>] [--title <t>] [--json]` prints to stdout. Events file format is the one `tally ingest` reads.

- [ ] **Step 1: Write the failing tests** (`hackatime.rs`; exact numbers):

```rust
#[test] fn gaps_under_the_timeout_join_and_longer_gaps_split() {
    let s = hackatime_spans(&[0, 60_000, 150_000, 400_000], 120_000);
    assert_eq!(s, vec![(0, 150_000), (400_000, 400_000)]);
}
#[test] fn agent_busy_span_generates_heartbeats_every_30s() {
    let hb = heartbeats_from_events(&[busy("a", 0, 3_600_000)]);   // helper builds Kind::AgentBusy
    assert_eq!(hb.len(), 121);
    assert_eq!(hackatime_spans(&hb, HACKATIME_TIMEOUT_MS), vec![(0, 3_600_000)]);
}
#[test] fn idle_events_are_not_heartbeats() {
    assert!(heartbeats_from_events(&[idle("i", 0, 600_000)]).is_empty());
}
```
- [ ] **Step 2:** Run `cargo test -p tally-evidence hackatime`. Expected: FAIL; implement; PASS.
- [ ] **Step 3: Write the failing tests** (`tests/replay.rs`; build sessions with helper constructors and compare through `compare`): (a) *agent runs while user is away*: one prompt at 0, `AgentBusy` 0 to 60 min, `Idle` from 5 min to 55 min, one prompt at 58 min. Assert `hackatime_s >= 3500`, `tally_verified_s < hackatime_s / 4`, `removed` non-empty, and at least one removed minute has reason `away`. (b) *no agent, steady human work*: prompts and edits every 2 min for 30 min. Assert `removed.len()` is at most 2 and `tally_verified_s > 0`. (c) *invariant*: for both sessions, `removed.len() * 60 + credited_common_minutes * 60` equals `hackatime_s` within one minute of rounding (expose `common_minutes` on `Report` and assert it).
- [ ] **Step 4:** Run; FAIL; implement `compare`, the renderers, and the CLI; PASS.
- [ ] **Step 5:** Generate three synthetic demo sessions with a small `examples/synth_sessions.rs` (agent-away, five-project afternoon, steady work), commit the NDJSON and their rendered reports under `docs/evidence/replay/`. A real teen's session is added by a human with their consent; leave `docs/evidence/replay/README.md` stating that these three are synthetic and must not be presented as real.
- [ ] **Step 6:** Commit: `feat(evidence): Hackatime-style vs Tally replay comparator`.

---

### Task 16: Red-team harness

**Files:**
- Create: `crates/tally-evidence/src/scenarios.rs`, `src/redteam.rs`, `tests/redteam.rs`
- Modify: `src/main.rs` (`redteam` subcommand)

**Interfaces:**
- `pub enum Layer { Scorer, Adapter, Judge }`: which part of the system the pitch relies on for that attack. The harness exercises the **scorer** with the events that layer would hand it; it does not run real harnesses.
- `pub struct Scenario { pub name: &'static str, pub pitch_row: &'static str, pub relies_on: Layer, pub assumption: &'static str, pub events: Vec<Event>, pub duration_s: i64 }`
- `pub fn all() -> Vec<Scenario>` and `pub fn control() -> Scenario` (honest hour of work).
- `pub struct Result { pub name, pub duration_s: i64, pub verified_s: i64, pub unverified_s: i64, pub reasons: Vec<String> }` and `pub fn run(s: &Scenario) -> Result` (just `score`, summed).
- Each scenario's `assumption` states, in one sentence, what the adapter must have done for these events to be the right input (for example "the Claude Code adapter emits no human prompt for a `claude -p` run because the log entry's origin is not human"). The report prints it, so a reader sees which defenses live outside the scorer.
- Scenarios, each 60 simulated minutes: 
  1. `scripted_claude_p_loop`: only `AgentBusy` and `AgentOutput` events, no human events. Relies on Adapter.
  2. `scripted_loop_misattributed_as_human`: a prompt every 60 s, `chars` 40, `relevance_pm` 0, `novelty_bytes` 0 (templated). Relies on Judge for templated detection; scorer-only credit reported anyway.
  3. `padded_random_prompts`: a prompt every 2 min, `chars` 4000, `relevance_pm` 0, `novelty_bytes` 3000.
  4. `planted_failing_test`: user edit plants a test (`UserEdit`), `TestRun` fail (`initiator: User`), `UserEdit`, `TestRun` pass with the same `cmd_hash`, repeated 12 times with fresh `cmd_hash` values. **Spec §8 defers the "failing test existed before the loop" check, so expect this attack to earn up to 1200 s per loop today.** The report must show that number and flag it as a known gap, not hide it.
  5. `esc_spam`: an `Interrupt` every 5 s, no prompts.
  6. `trivial_commits`: a `Commit { by: User, lines_changed: 1 }` every 90 s.
  7. `parallel_agents`: five overlapping 60-minute `AgentBusy` spans plus agent outputs; no human events.
  8. `idle_youtube_while_agent_runs`: `AgentBusy` for 60 minutes, `Idle` from minute 3 to 58, one honest prompt at minute 0 and one at minute 59.
  9. `hourly_bot_push` is server-side (plan 09): not modelled; listed in the report as `not modelled here`, with the plan that covers it. Likewise `prompt_injection_in_transcript` (plan 10).
- Report: Markdown table `attack | relies on | simulated minutes | credited minutes | credited as % of honest control | assumption`, then a section per attack with its reasons, then a **Gaps** list that names every attack whose credited minutes exceed 5 percent of the duration. The output starts with the ruleset version and the commit hash of `tally-scoring`.

- [ ] **Step 1: Write the failing tests** (`tests/redteam.rs`):

```rust
#[test] fn control_earns_real_credit_so_a_zero_scorer_cannot_pass() {
    let r = run(&control());
    assert!(r.verified_s >= 1800, "honest hour credited {} s", r.verified_s);
}
#[test] fn attacks_with_no_human_events_earn_nothing() {
    for n in ["scripted_claude_p_loop", "parallel_agents"] {
        assert_eq!(run(&by_name(n)).verified_s, 0, "{n}");
    }
}
#[test] fn padded_prompts_are_capped_per_claim() {
    let s = by_name("padded_random_prompts");
    let r = run(&s);
    let prompts = s.events.len() as i64;
    assert!(r.verified_s <= prompts * 90, "{}", r.verified_s);     // trivial/zero-relevance cap is 90 s (spec §4)
}
#[test] fn youtube_while_agent_runs_credits_only_the_edges() {
    assert!(run(&by_name("idle_youtube_while_agent_runs")).verified_s <= 600);
}
#[test] fn every_scenario_names_its_assumption_and_pitch_row() {
    for s in all() { assert!(!s.assumption.is_empty() && !s.pitch_row.is_empty(), "{}", s.name); }
}
```
- [ ] **Step 2:** Run `cargo test -p tally-evidence redteam`. Expected: FAIL. Implement the generators with a deterministic builder (fixed ids `rt:<scenario>:<n>`, no randomness; "random" prompts are modelled by their derived numbers, which is all the scorer sees).
- [ ] **Step 3:** Run. If a bound above fails because the scorer credits more, **do not loosen the test silently**: the number is a finding. Record the measured value in the test as the new bound with a comment naming the spec gap, and list it under Gaps in the report.
- [ ] **Step 4:** Implement `redteam` CLI: `tally-evidence redteam [--out docs/evidence/redteam.md]`. Run it; commit the generated `docs/evidence/redteam.md`.
- [ ] **Step 5:** Commit: `feat(evidence): red-team harness scoring every threat-model attack`.

---

### Task 17: Resource measurement harness

**Files:**
- Create: `crates/tally-evidence/src/load.rs`, `src/resources.rs`, `tests/resources.rs`, `docs/evidence/README.md`
- Modify: `crates/tally-evidence/Cargo.toml` (`sysinfo = "0.32"`), `src/main.rs` (`load`, `resources` subcommands)

**Goal:** the pitch's "under 50 MB of RAM and under 1% CPU during active use, measured on a real 2016 Windows laptop", as a repeatable command that writes a numbers file. The measurement itself is run by a human on the real laptop (Step 6).

**Interfaces:**
- `load.rs`: `pub struct LoadPlan { pub hours: u32, pub prompts_per_hour: u32, pub tool_events_per_prompt: u32, pub seed: u64 }` and `pub fn write_session_log(dir: &Path, plan: &LoadPlan, clock: &mut dyn Clock) -> io::Result<()>`: appends lines to a session log under `dir`, in the **Claude Code log format fixture that plan 03's Task 1 spike captured** (copy its line shapes from `crates/tally-daemon/tests/fixtures/`; do not invent a format), at realistic cadence (default 8 h simulated, 40 prompts per hour, 6 tool events per prompt). Two modes: `--real-time` (writes at wall-clock cadence, for the laptop run) and `--accelerated N` (N times faster, for CI smoke).
- `resources.rs`: `pub struct Sample { pub t_s: u64, pub rss_bytes: u64, pub cpu_pct_x100: u32 }`; `pub fn summarize(samples: &[Sample]) -> Summary { rss_p50, rss_p95, rss_max, cpu_mean_x100, cpu_p95_x100, cpu_max_x100, n }` (pure); `pub fn measure(child_pid: u32, duration: Duration, every: Duration) -> Vec<Sample>` using `sysinfo` for that pid only (the harness and the load writer are other processes and are excluded); `pub fn render_numbers(s: &Summary, machine: &MachineInfo, load: &LoadPlan, version: &str) -> String` (Markdown).
- `MachineInfo { description: String /* from --machine, required */, os, cpu_model, logical_cores, total_ram_bytes }` (the last four read through `sysinfo`).
- CLI: `tally-evidence resources --tally <path-to-tally-binary> --machine "<free text>" --duration-min 60 --load real-time --out docs/evidence/resources-<slug>.md`. It creates a temporary home and a temporary harness log directory, writes a `config.toml` pointing the Claude adapter at it, spawns `tally run --home <tmp>`, starts the load writer, samples once a second, kills the daemon, and writes the numbers file. **It refuses to write the file when `--machine` is missing.** Phases are reported separately: `idle` (first 10 minutes, no load) and `active` (rest); the 50 MB and 1% targets are checked against `active`, and the file states pass or fail for each next to the measured figure.

- [ ] **Step 1: Write the failing tests** (`tests/resources.rs`, pure):

```rust
#[test] fn summary_percentiles_and_means() {
    let s: Vec<Sample> = (1..=100).map(|i| Sample { t_s: i, rss_bytes: i * 1_000_000, cpu_pct_x100: (i % 4) as u32 * 100 }).collect();
    let r = summarize(&s);
    assert_eq!((r.rss_p50, r.rss_p95, r.rss_max), (50_000_000, 95_000_000, 100_000_000));
    assert_eq!(r.cpu_max_x100, 300);
    assert_eq!(r.cpu_mean_x100, 150);
}
#[test] fn numbers_file_states_pass_or_fail_against_the_targets() {
    // Summary with rss_p95 = 60 MB, cpu_p95 = 0.4 percent: output contains "RSS p95" and "FAIL" for memory, "PASS" for cpu
}
#[test] fn missing_machine_description_is_refused() {
    // run the CLI function with machine = "" -> Err, and no file written
}
```
- [ ] **Step 2:** Run `cargo test -p tally-evidence resources`. Expected: FAIL; implement the pure parts; PASS.
- [ ] **Step 3:** Implement `load.rs` against the plan 03 fixture shapes; test: write 1 simulated hour accelerated 1000x into a temp dir, then run the real `tally` adapter over it (`tally run --once`, or the plan 03 equivalent) and assert it ingests a non-zero event count and the log lines parse with no adapter warnings.
- [ ] **Step 4:** Implement `measure` and the `resources` CLI. CI smoke (all three OSes): `tally-evidence resources --tally target/release/tally --machine "ci" --duration-min 1 --load accelerated:600 --out $RUNNER_TEMP/r.md`; assert the file exists and contains `RSS p95`. CI numbers are never committed as published figures (the workflow does not upload them to `docs/evidence`).
- [ ] **Step 5:** Write `docs/evidence/README.md`: how to run on the target laptop. The checklist: a physical machine from about 2016 (for example 4 to 8 GB RAM, dual-core CPU, spinning disk or SATA SSD, Windows 10 or 11 as it ships to users), plugged in and on battery (two runs), no other heavy applications, antivirus left on (it is what a teen has), release build of `tally` from the release workflow, a 60-minute `--real-time` run, then commit the generated numbers file. Record the laptop make, model, year, CPU, RAM and Windows build in `--machine`.
- [ ] **Step 6 (human):** Run the checklist on the real 2016 laptop and commit `docs/evidence/resources-<slug>.md`. If either target fails, the numbers file says so and the maintainer decides whether to fix or restate the pitch; do not edit the figure.
- [ ] **Step 7:** Commit: `feat(evidence): resource measurement harness`.

---

### Task 18: Cost sheet generator from measured ledger sizes

**Files:**
- Create: `crates/tally-evidence/src/cost.rs`, `tests/cost.rs`, `docs/evidence/cost-inputs.toml`
- Modify: `src/main.rs` (`cost` subcommand)

**Interfaces:**
- Input 1: a published per-project ledger JSON (the file plan 05's `tally publish` writes: a JSON array of `LedgerEntry`). Input 2: `cost-inputs.toml`, which holds **prices the maintainer supplies and the generator never invents**: `users_monthly = 15488`, `hours_monthly = 109759` (from PITCH), `[prices]` entries as `{ item, low_usd, high_usd, source }` for API and Postgres hosting, GPU-hours for the judge and its hours range, object storage, signing and Apple account. A price with an empty `source` makes the generator print `UNSOURCED` next to it.
- `pub struct LedgerStats { pub entries: usize, pub verified_s: u64, pub bytes_total: u64, pub bytes_per_entry_mean: u64, pub bytes_per_verified_hour: u64 }`; `pub fn ledger_stats(entries_json: &str) -> Result<LedgerStats>`. Bytes are the canonical JSON length of each entry (the same encoding the server stores, `serde_json::to_string`).
- `pub fn project(stats: &LedgerStats, hours_monthly: u64) -> Projection { entries_monthly, upload_bytes_monthly, storage_bytes_yearly }`.
- `pub fn render(stats, projection, inputs) -> String`: Markdown with three sections: measured (from the ledger, with the source file and number of entries), projected (monthly volume at PITCH scale), and the line-item cost table with totals low and high. The measured numbers replace the PITCH sentence "tens of bytes per action record" if they differ; the output says so when the measured mean exceeds 100 bytes.

- [ ] **Step 1: Write the failing tests:**

```rust
#[test] fn stats_over_two_known_entries() {
    // two LedgerEntry JSONs built with tally_scoring::chain over fixed claims; verified_s 600 + 300
    let st = ledger_stats(&json).unwrap();
    assert_eq!((st.entries, st.verified_s), (2, 900));
    assert_eq!(st.bytes_total, (a.len() + b.len()) as u64);
}
#[test] fn projection_scales_by_verified_hours() {
    let st = LedgerStats { entries: 10, verified_s: 3600, bytes_total: 2000, bytes_per_entry_mean: 200, bytes_per_verified_hour: 2000 };
    let p = project(&st, 109_759);
    assert_eq!(p.upload_bytes_monthly, 109_759 * 2000);
}
#[test] fn unsourced_price_is_flagged_and_totals_use_only_given_numbers() {
    // inputs with one price missing `source`: output contains "UNSOURCED"; low total equals the sum of low_usd values
}
#[test] fn empty_ledger_is_an_error_not_a_divide_by_zero() { assert!(ledger_stats("[]").is_err()); }
```
- [ ] **Step 2:** Run `cargo test -p tally-evidence cost`. Expected: FAIL; implement; PASS.
- [ ] **Step 3:** Run on this repo's real published ledger once plan 05 has produced one: `tally-evidence cost --ledger <path> --inputs docs/evidence/cost-inputs.toml --out docs/evidence/cost.md`. Commit the output. Before that ledger exists, run it on a ledger from `tally-evidence`'s synthetic sessions and name the file `cost-synthetic.md`.
- [ ] **Step 4:** Commit: `feat(evidence): cost sheet from measured ledger sizes`.

---

## Done when

- `cargo test --workspace` and clippy pass on Windows, macOS and Linux CI; the presence probes compile everywhere and are exercised for real on at least Windows plus one macOS and one Linux machine, with every unverified environment marked `NOT VERIFIED` in `docs/platform-notes.md`.
- On a desktop with no idle support, `tally status` says unsupported and the ledger gains no entries.
- `tally init` on a clean machine prints the explainer, finds Claude Code, registers the current repo and writes the config once; `tally service install/status/stop/uninstall` works on all three OSes and uninstall leaves `<home>` intact.
- A second `tally run` on the same home is refused.
- A tagged dry run produces six archives and `SHA256SUMS`; both installers pass the run-twice CI test.
- `docs/evidence/` contains the red-team report (with its gaps listed), the synthetic replay samples, and a resource numbers file measured on a real low-spec laptop (human step).

## Not in this plan

- Running the pilot, blind grading, the real teen's replay, or the red-team with live harnesses: human work.
- The judge-dependent defenses (templated prompts, injection) and server-side ones (hourly bot pushes): plans 09 and 10 own them; the red-team report lists them as not modelled.
- The "failing test existed before the loop" rule (spec §8, ruleset change): the red-team report quantifies the gap, a ruleset plan fixes it.
- Auto-update, a GUI installer, `.deb`/`.rpm`/MSI packages, Windows Service mode.
- Web-builder browser extension.

## Decisions I made (maintainer to confirm)

1. **User-session background task, not a Windows Service.** Idle input is per interactive session; a service in session 0 cannot read it. Task Scheduler logon task, launchd agent, systemd user unit (`graphical-session.target`).
2. **Unsupported means no finalization.** An unsupported or failing presence probe pins the presence watermark at 0 so the horizon never advances; nothing is scored into the ledger. I chose this over "assume present" because the scorer over-credits missing idle data.
3. **Wayland takes priority over X11** whenever a Wayland session is detected (XWayland's XScreenSaver counter only sees X clients). `ext-idle-notify` only gives 30 s-resolution idle, which matches `IDLE_SAMPLE_MIN`.
4. **Installers do not start tracking.** They install the binary only; consent, the explainer and service registration happen in `tally init`.
5. **`tally-evidence` is a separate, unshipped crate** so evidence tooling never bloats the 50 MB budget binary.
6. **Linux builds are static musl**; Windows and macOS native, six targets.
7. **Hackatime comparator is an approximation** (all non-idle events as heartbeats, 2-minute timeout) unless real Hackatime heartbeats are supplied.
8. **Red-team harness drives the scorer with the events each adapter would emit**, and prints the assumption per attack; it reports gaps (the planted-failing-test attack currently earns credit) rather than tuning tests to pass.
9. Presence trait names (`IdleProbe`, `ProbeError`, `PresenceSource`) are guessed because plan 04's file did not exist; reconcile first.

## Questions only the maintainer can answer

- Windows signing route (PFX certificate versus a cloud signing service) and the Apple Developer account owner; the secret names in Task 12 assume a PFX and a Developer ID certificate.
- The timeout Hackatime actually uses, and whether a real Hackatime heartbeat export is available for the replay.
- Which real 2016 Windows laptop will be measured, and whether the Task 17 numbers get published if a target is missed.
