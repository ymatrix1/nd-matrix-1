---
# ns4-u8o2
title: Move firmware-derived tables and code images to runtime loading
status: todo
type: task
created_at: 2026-11-17T04:06:20Z
updated_at: 2026-11-23T06:00:00Z
parent: ns4-trkb
---

**Context**
D128 classifies tracked firmware-derived data (docs/research/firmware-derived-data-inventory.md). Classes B (code images) and C (large tables) are still committed.

**Higher Goal**
Repository holds only our own work; firmware data is read from the user OS update file at runtime.

**Acceptance Criteria**
- [ ] Loader for core DSP images via FirmwareContainer::extract_segment
- [ ] Default render without an image has a documented fallback
- [ ] Each path below is migrated to runtime loading with a synthetic-fixture test, then deleted and removed from `tools/fw-derived-baseline.txt` (the ratchet in `tools/check_fw_derived.py` only allows the list to shrink). Verification needs the owner's OS update file, which is not available in the agent environment.
- [ ] `crates/nord-core/src/organ/b3_bass_fw/prog_gen.rs`
- [ ] `crates/nord-core/src/organ/b3_bass_fw/prog_proc.rs`
- [ ] `crates/nord-core/src/organ/farfisa_fw/prog_proc.rs`
- [ ] `crates/nord-core/src/organ/firmware_bank.rs`
- [ ] `crates/nord-core/src/organ/vox_image.rs`
- [ ] `docs/research/data/bass-fw-image.csv`
- [ ] `docs/research/data/farfisa-fw-image.csv`
- [ ] `crates/nord-core/src/engine/zone_table.rs`
- [ ] `crates/nord-core/src/fx/fw_coeffs.rs`
- [ ] `crates/nord-core/src/fx/fw_delay_table.rs`
- [ ] `crates/nord-core/src/fx/fw_drive_tables.rs`
- [ ] `crates/nord-core/src/fx/fw_eq_tables.rs`
- [ ] `crates/nord-core/src/fx/fw_lp24_tables.rs`
- [ ] `crates/nord-core/src/fx/fw_mod2_dense.rs`
- [ ] `crates/nord-core/src/fx/fw_mod2_tables.rs`
- [ ] `crates/nord-core/src/fx/tables.rs`
- [ ] `crates/nord-core/src/synth/fw_filter_tables.rs`
- [ ] `crates/nord-core/src/synth/fw_ladder_tables.rs`
- [ ] `crates/nord-core/src/synth/fw_stage3_tables.rs`
- [ ] `docs/research/arm-chan-tables.csv`
- [ ] `docs/research/arm-param-table.csv`
- [ ] `docs/research/data/fx-mod2-tables.csv`
- [ ] `docs/research/data/fx-type-words.csv`
- [ ] `docs/research/data/oled-strings-vs-firmware.csv`
- [ ] `docs/research/data/synth-env-rate-table.csv`
- [ ] `docs/research/data/synth-env-rates-settled.csv`
- [ ] `docs/research/data/synth-filter-t3-tables.csv`
- [ ] `docs/research/data/synth-filter-table-d.csv`
- [ ] `docs/research/data/synth-filter-tables.csv`
- [ ] `docs/research/data/synth-fm-tables.csv`
- [ ] `docs/research/data/synth-knob-laws.csv`
- [ ] `docs/research/data/synth-ladder-coef-table.csv`
- [ ] `docs/research/data/synth-ladder-shaper-table.csv`
- [ ] `docs/research/data/synth-p-words.csv`
- [ ] `docs/research/data/synth-super24-tables.txt`
- [ ] `docs/research/data/synth-sync-table.json`
- [ ] `docs/research/data/synth-velocity-law.csv`
- [ ] `docs/research/data/synth-wave-types.csv`

**Out of Scope**
History rewrite (owner decision). The ratchet itself (bean `ns4-r4tq`, done).

**Summary of Changes**
(pending)
