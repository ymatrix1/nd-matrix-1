# Build and benchmark speed

Checked: 2026-11-28.
Sources: `Cargo.toml` profiles, `crates/nord-sharc/src/cpu_clock.rs`, `tools/bench_ab.py`, timing runs in this note, `docs/research/sharc-lowering-v2.md` section 8, `docs/research/sharc-kernel-speed.md`.
Question: where does the implement-measure loop lose time, and which cheap changes shorten it without weakening exactness or the measurement rules?

## Host and method

Windows 11, Ryzen 9 5900XT, 32 logical cores. Three other sessions were building and benchmarking during these runs, so wall-clock figures are EMPIRICAL and rough (one to three runs each). Baseline = the tree before this change (opt-level 0, full debug info), built in a separate target dir with `--config` overrides; new = this change.

## Results

| What | Before | After | Note |
|---|---|---|---|
| `cargo test -p nord-core -p nord-sharc -p nord-arm`, sum of the 28 suite durations | 76.5 s (largest suite 46.0 s) | 19.8 s (largest 9.5 s) | all suites pass in both |
| Rebuild of the three crates after touching all of them (deps warm) | 30.1 s, 25.7 s | 26.9 s, 24.7 s | no cost, within noise |
| Incremental rebuild after touching `nord-core/src/lib.rs` | 16.2, 17.2, 16.4 s | 18.6, 16.4, 16.9 s | within noise |
| `nord-sharc` rebuild with the ORG2 whole-pass module, `--release` vs a `codegen-units = 32`, `incremental = false` profile | 94.9 s | 91.3 s | one run each, within noise: the profile was **not adopted** |

What did not help: spreading the AOT module over more codegen units (above). The AOT compile is bound by the size of the generated module, not by the number of units. Cutting it needs less generated code (fewer inlined sites, cold paths out of line), measured in `rustc` seconds and code bytes per step as `sharc-lowering-v2.md` already requires.

Not measured: the effect of `debug = "line-tables-only"` on link time (the rebuild rows above did not move), `sccache` across worktrees (a dependency cache; every new worktree rebuilds all dependencies from scratch, `sccache` is installed locally but `RUSTC_WRAPPER` is left unset: it changes how every session builds, so it stays the user's call).

## Measurement

`nord_sharc::cpu_clock::ThreadClock` reads the thread CPU time (`QueryThreadCycleTime` on Windows x86-64, converted with a one-time TSC calibration; wall time elsewhere, reported by `ThreadClock::is_cpu_time()`). It now times:

- the per-core pass in `runner::pass_sched` (`CoreSeries.ns`; the barrier tick stays wall time),
- the per-tick rows of `arm_lle_bench`,
- `kernel_bench` (moved from its private copy).

Time a thread spends descheduled by another build is not counted, so a run on a loaded host is usable with 3 to 5 interleaved rounds instead of 7 to 9 on a quiet host. Residual noise: cache and SMT contention, memory bandwidth, frequency. Wall time stays the rule for anything with a real-time deadline (the paced ARM producer, the barrier tick) and for the final claim "x of budget": a CPU-time figure is a lower bound of the wall time on a loaded host.

`tools/bench_ab.py --a CMD --b CMD --metric REGEX` alternates A and B (order swapped every round), prints median [min..max] per side, the B/A ratio and "within noise" unless the bands are disjoint, and records the number of `cargo`/`rustc`/`link` processes before every round.

## Protocol after this change

1. Gate a stage on deterministic counters first (steps, blocks, instructions, bit-exact diff). They need no repeats.
2. Run `bench_ab.py` with a CPU-time metric, 3 to 5 rounds, any host load; rounds that overlapped a compiler are listed.
3. Quote the wall-clock figure for the real-time budget only once, on a quiet host, with 7 to 9 rounds.

## Implications

- Test runs of the emulator crates are 3.9x faster in dev; nothing else got slower in the measured rows.
- A/B decisions need fewer rounds and no waiting for a quiet host; the final budget claim still does.

## Follow-ups

- Measure `sccache` (and a shared `CARGO_TARGET_DIR`) on a quiet host; decide whether to document `RUSTC_WRAPPER` in `AGENTS.md`.
- Per-image AOT crates (one `rustc` per image, parallel) and fewer inlined sites to cut the 95 s module compile.
- `lle_sharc_bench` still prints one `mean` per core; add the CPU/wall pair to its report line and use `--metric` on it.
