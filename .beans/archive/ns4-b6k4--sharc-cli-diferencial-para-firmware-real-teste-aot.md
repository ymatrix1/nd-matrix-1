---
# ns4-b6k4
title: 'SHARC: CLI diferencial para firmware real + teste AOT vs oraculo'
status: completed
type: task
priority: normal
created_at: 2026-11-16T17:58:49Z
updated_at: 2026-11-16T18:03:57Z
parent: ns4-trkb
---

Added real-image differential CLI/runner (sharc-jit-diff + runner::jit_diff) and compiled AOT-vs-Core::run oracle equivalence test.

## Summary of Changes
- crates/nord-sharc/src/core/sysregs.rs: made Core::astat pub.
- crates/nord-sharc/src/runner/mod.rs & crates/nord-sharc/src/runner/jit_diff.rs: added compare_full_state, diff_run, and unit tests (+ ignored NORD_SHARC_ORG_IMAGE real-image test).
- crates/nord-sharc/src/bin/sharc_jit_diff.rs: CLI comparing Core::run vs Core::run_jit every N steps and stopping on first divergence.
- crates/nord-sharc/src/core/aot.rs: added aot_compiled_rustc_matches_core_run_oracle compiling transpiled Rust with rustc -O and verifying bit-exact match with Core::run.
