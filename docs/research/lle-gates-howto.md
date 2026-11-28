# lle-gates: measuring the LLE gates

- **Checked:** 2026-11-28
- **Sources:** `crates/nord-tools/src/bin/lle_gates.rs` and `lle_gates/`, reusing `runner::uop_report::measure`, `lle_sharc_bench/job.rs` (booting and arming), `arm_bench::run_all` (the `arm-lle-bench` phases), `tools/uop_gates.py`; gate rows from [sharc-compiler-design.md](sharc-compiler-design.md) section 12 and [arm0-jit-design.md](arm0-jit-design.md) section 13.
- **Question:** how do we take the G2/G3 numbers of the SHARC uop engines and the ARM0 tick loop in one run, on one host, without fooling ourselves?

## Run it on the owner's machine

Real timings belong on the owner's machine, not in a cloud container (shared, throttled, noisy CPU). The tool refuses when the host is busy, but a quiet cloud VM is still not the target hardware.

```bash
cargo +1.100.0 run --release -p nord-tools --bin lle-gates -- --fw .reference/fw_unpacked --bank 0 \
  --json lle_gates.json --uop-reports uop_reports.json
python tools/uop_gates.py uop_reports.json
```

Native tier: add `--features nord-sharc/uop-native` (SHARC `uop-native`). Without it the row says why it is missing. No firmware, wiring check only: `--synthetic`.

Close other programs first. Do not run the tool next to a build, a browser or a DAW.

## Files it reads (ASSUMED layout, override with options)

| What | Default path under `--fw` | Option |
|---|---|---|
| ORG Core2 raw SRAM image | `ORG/core2_sram.bin` | `--org-image` |
| PNO Core2 raw SRAM image | `PNO/core2_sram.bin` | `--pno-image` |
| ARM0 OS code, bank N | `bank<N>_CODE.bin` | `--bank` |
| ARM0 OS SRAM, bank N | `bank<N>_SRAM.bin` | `--bank` |
| ARM0 EEPROM image | `eeprom_image_256k.bin` | `--eeprom` |

`--fw` defaults to `.reference/fw_unpacked` (git-ignored; nothing is committed, D128). A missing file skips that part with a message naming the path and exits 0; with nothing to measure the JSON `status` is `skipped`. The AOT row needs `--aot-tag-org` / `--aot-tag-pno` (the tag of the embedded whole-pass module), otherwise it reports `n/a`.

## What is measured

| Part | Engines or tiers | Per pass or tick |
|---|---|---|
| SHARC ORG Core2, PNO Core2 | `interp`, `aot`, `uop` (tier 1), `uop-checked`, `uop-native` | wall and thread-CPU microseconds |
| ARM0 six-phase tick loop | `plain` (`Cpu::step`), `predecode`; `sb`, `sb-native` listed as unavailable | wall microseconds |

`uop-checked` replays every pass on the interpreter and allocates: it contributes counters and the number of passes compared, never a time. The superblock ARM tiers cannot be selected on a `Machine` until the seam lands; the tool says so instead of measuring something else.

Per engine the report gives mean, median, p99 and max per pass, each as median [min..max] over the runs; the share of the 46.08 us pass budget (or the 1 ms tick); for uop engines steps per pass, the interpreter share of steps, `maintain_calls` and `smc` of the measured passes (G2); for ARM the share of ticks under 1 ms.

## Measurement discipline

1. **Load.** The background CPU load is sampled while the tool itself sleeps (so it is other processes only): before everything, before every run, after everything. Linux reads `/proc/stat`, Windows `GetSystemTimes`; elsewhere the load is unknown and unknown counts as loaded. Limit 25 % (`--load-limit`).
2. **Refusal.** Above the limit before the start: no timing is taken (exit 4). Above it during or after: the timings are withheld, counters stay. `--indicative` prints the numbers marked INDICATIVE with no gate verdict.
3. **Alternation.** Runs go engine by engine in rounds (interp, aot, uop, ... then again), so drift hits all engines alike. Round 0 is discarded; each run also drops `--warmup` passes. Default 7 measured rounds (ASSUMED, as `arm_sb_ab`).
4. **Bands and noise.** The figure compared is the median of the per-run medians; its band is min..max over runs. Two engines whose medians differ by less than the larger band print `within noise` and no ratio.
5. **Gates.** Counts (G2, K3) are evaluated by `tools/uop_gates.py` from `--uop-reports`, which holds counts only. The tool evaluates the wall-clock rows itself: G3 (uop at least x1.4 over AOT with ranges apart, ORG pass <= 115 us) and G3-RT (pass <= 1.3 x 46.08 us). Thresholds are the design tables' numbers, ASSUMED decision thresholds, not measurements.

Exit codes: 0 report or clean skip, 2 usage, 3 a run did not return to its poll loop, 4 timings refused for host load.

## Implications

- One command produces the table, the JSON and the counter file for every gate that needs a clock; nothing is copied from the existing benches.
- A "faster" claim in a PR can quote this output, including its load line and bands.

## Follow-ups

- Hook for the ARM superblock tiers: a `Machine`-level switch for `SbConfig` honoured by `run_tick_idle`, then `arm::unavailable` can go.
- ARM G2/G3 rows (tier-2 A/B, exits share) once those tiers can run.
- Real numbers from the owner's machine go into [OPEN-QUESTIONS.md](../OPEN-QUESTIONS.md) / the design notes, not into this file.
