# Build and benchmark speed

Checked: 2026-11-28 (reviewed and corrected the same day).
Sources: `Cargo.toml` profiles, `crates/nord-sharc/src/cpu_clock.rs`, `tools/bench_ab.py`, timing runs in this note, `docs/research/sharc-lowering-v2.md` section 8, `docs/research/sharc-kernel-speed.md`.
Question: where does the implement-measure loop lose time, and which cheap changes shorten it without weakening exactness or the measurement rules?

## Host and method

Windows 11, Ryzen 9 5900XT, 32 logical cores. Several other sessions (three to seven) were building and benchmarking during these runs, so wall-clock figures are EMPIRICAL and rough (one to three runs each). Baseline = the tree before this change (opt-level 0, full debug info), built in a separate target dir with `--config` overrides; new = this change.

## Results

| What | Before | After | Note |
|---|---|---|---|
| `cargo test -p nord-core -p nord-sharc -p nord-arm`, sum of the 28 suite durations | 76.5 s (largest suite 46.0 s) | 19.8 s (largest 9.5 s) | all suites pass in both |
| Rebuild of the three crates after touching all of them (deps warm) | 30.1 s, 25.7 s | 26.9 s, 24.7 s with the first version of the change; 20.2, 20.4, 19.4 s with the final one (no `debug` setting) | no cost; the last three rows ran later at a different host load, so only "not slower" is claimed |
| Incremental rebuild after touching `nord-core/src/lib.rs` | 16.2, 17.2, 16.4 s | 18.6, 16.4, 16.9 s | within noise |
| `nord-sharc` rebuild with the ORG2 whole-pass module, `--release` vs a `codegen-units = 32`, `incremental = false` profile | 94.9 s | 91.3 s | one run each, within noise: the profile was **not adopted** |

What did not help: spreading the AOT module over more codegen units (above). The AOT compile is bound by the size of the generated module, not by the number of units. Cutting it needs less generated code (fewer inlined sites, cold paths out of line), measured in `rustc` seconds and code bytes per step as `sharc-lowering-v2.md` already requires.

Not adopted: `debug = "line-tables-only"` (the first version of the change had it; no benefit was measurable, so it was dropped), and `sccache` across worktrees (a dependency cache; every new worktree rebuilds all dependencies from scratch, `sccache` is installed locally but `RUSTC_WRAPPER` is left unset: it changes how every session builds, so it stays the user's call).

## Measurement

`nord_sharc::cpu_clock::ThreadClock` reads the thread CPU time (`QueryThreadCycleTime` on Windows x86-64, converted with a one-time TSC calibration; monotonic wall time elsewhere, reported by `ThreadClock::is_cpu_time()`). Both clocks are recorded; **wall time stays the figure for every deadline and every "x of budget" claim**:

- `runner::pass_sched`: `CoreSeries.ns` is wall ns (unchanged meaning), `CoreSeries.cpu_ns` is new; `lle_sharc_bench` prints "mean/budget" from wall and a `cpu mean us` column next to it. The barrier tick stays wall. The calibration (200 ms, first use) runs once before the core threads start.
- `arm_lle_bench`: the tick statistics (p50/p99/max, `>1ms`, backlog) are wall; the new `cpu mean/max ms` columns are CPU. The header line says which clock the CPU column is.
- `kernel_bench` (CPU time, as before; now through the shared module).

A CPU-time figure is not inflated by the thread being descheduled by another build, so it is the comparable number for A/B work on a loaded host. It is a lower bound of the wall time on a loaded host, so it cannot support a real-time claim. ASSUMED: that this lets an A/B decision use fewer rounds than the 7 to 9 of `sharc-lowering-v2.md` section 8. No comparison of the CPU-time and wall-time spread under load was made; it would need 5 runs of `kernel_bench`-style loops with both clocks recorded, on a loaded and on a quiet host. Until then keep 7 to 9 rounds for a decision, and use fewer only for exploration.

`tools/bench_ab.py --a CMD --b CMD --metric REGEX` alternates A and B (order swapped every round), prints median [min..max] per side, the B/A ratio and "within noise" unless the bands are disjoint, and records the number of `cargo`/`rustc`/`link` processes before every round. For `lle_sharc_bench` use `--metric "total CPU per pass ([0-9.]+) us"`.

## Protocol after this change

1. Gate a stage on deterministic counters first (steps, blocks, instructions, bit-exact diff). They need no repeats.
2. Run `bench_ab.py` on a CPU-time metric for exploration (any host load; rounds that overlapped a compiler are listed); 7 to 9 rounds for a decision until the spread comparison above is done.
3. Quote the wall-clock figure for the real-time budget once, on a quiet host, with 7 to 9 rounds.

## Implications

- Test runs of the emulator crates are 3.9x faster in dev; nothing else got slower in the measured rows.
- A/B exploration can run on a loaded host with the CPU-time column; decisions and the final budget claim still need the wall-clock rules.

## Follow-ups

- Measure `sccache` (and a shared `CARGO_TARGET_DIR`) on a quiet host; decide whether to document `RUSTC_WRAPPER` in `AGENTS.md`.
- Per-image AOT crates (one `rustc` per image, parallel) and fewer inlined sites to cut the 95 s module compile.
- Measure the CPU-time versus wall-time spread under load (above) and then lower the round count in the protocol.
