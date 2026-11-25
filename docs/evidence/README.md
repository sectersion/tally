# Evidence

Outputs of `tally-evidence` (`crates/tally-evidence`, never shipped to users). Studies (the pilot, blind grading, reviewer verdicts) are human work; this directory holds what the tooling produces.

| File | Made by | Status |
|---|---|---|
| `replay/` | `tally-evidence replay`, `examples/synth_sessions.rs` | three **synthetic** sessions; see `replay/README.md` |
| `redteam.md` | `tally-evidence redteam --out docs/evidence/redteam.md` | generated; lists gaps the scorer leaves open |
| `resources-<slug>.md` | `tally-evidence resources` on the real laptop | **not yet measured**: needs the human step below |
| `cost*.md` | `tally-evidence cost` | see `cost-inputs.toml` |

## Measuring "under 50 MB of RAM and under 1% CPU" on the real laptop (human step)

The pitch's number is for a real 2016 Windows laptop. A CI run or a developer machine is a smoke test, never the published figure: accelerated runs say so in the numbers file.

Checklist:

1. A physical machine from about 2016 (for example 4 to 8 GB RAM, a dual-core CPU, spinning disk or SATA SSD), running Windows 10 or 11 as it ships to users.
2. No other heavy applications. **Antivirus left on** (it is what a teen has).
3. A release build of `tally` from the release workflow (the `-unsigned` archive is fine), unpacked somewhere; `tally-evidence` built with `cargo build --release -p tally-evidence` (or copy the binary).
4. Two runs, one plugged in and one on battery:

   ```
   tally-evidence resources --tally C:\path\to\tally.exe --machine "<make, model, year, CPU, RAM, Windows build>" --duration-min 60 --load real-time --plugged --out docs\evidence\resources-<slug>-plugged.md
   tally-evidence resources --tally C:\path\to\tally.exe --machine "<same>" --duration-min 60 --load real-time --battery --out docs\evidence\resources-<slug>-battery.md
   ```

   `--machine` is required: the tool refuses to write numbers without it.
5. Commit the generated files as they are. If a target fails, the file says `FAIL` next to the measured figure; the maintainer decides whether to fix the daemon or restate the pitch. **Do not edit the figure.**

How to read the file:

- The first tenth of the run (at most 10 minutes) is the `idle` phase with no load, the rest is `active`; the targets are checked against `active`.
- RSS and CPU are the daemon process's own (`tally run`); the harness and the load writer are other processes and are excluded.
- CPU is percent of **one** core as `sysinfo` reports it, which is the conservative reading on a dual-core machine. The first seconds after start contain a catch-up burst, so look at the mean as well as p95.
- Presence stays on in a real run. `--no-presence` exists for CI runners that have no idle source.

CI runs `tally-evidence resources --duration-min 1 --load accelerated:600 --no-presence` on all three OSes only to prove the harness works. Those numbers are not uploaded or committed.
