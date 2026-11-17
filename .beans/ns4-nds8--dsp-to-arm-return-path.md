---
# ns4-nds8
title: DSP to ARM return path
status: in-progress
type: task
created_at: 2026-11-17T17:23:01Z
updated_at: 2026-11-17T17:23:01Z
---

Context: ARM reads DSP memory by SPI; harness served a test pattern. Higher Goal: live ARM-DSP link return channel (D147). Acceptance Criteria: - [x] inventory - [x] opt-in return source + tests - [x] end to end runs. Out of Scope: SY live chip, defaults. Summary of Changes: tools/arm_dsp_return*.py, docs/research/dsp-to-arm-return.md
