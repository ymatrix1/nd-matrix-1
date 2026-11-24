---
# ns4-agyp
title: 'FX parity: combined FX ablation of the worst 20 and the PARITY preset'
status: completed
type: task
priority: normal
created_at: 2026-11-18T04:27:02Z
updated_at: 2026-11-18T04:52:38Z
parent: ns4-trkb
---


## Context
Program ranking (D160, PR #519) scored the worst 20 programs by single FX group reverts; for 6 of them the best single group left more than 5. #515/D158: every FX port that runs on the chip nulls the oracle; the engine default path diverges.

## Higher Goal
Audio 1:1 with the hardware from firmware and oracle evidence only; find which FX combination carries each program's gap and make the verified path one switch away.

## Acceptance Criteria
- [x] program_rank --ablate-fx (63 FX combinations) and --flip (default with groups switched to firmware); tools/program_rank_combo.py pairs, greedy, best set, flip tables
- [x] CSV + research note: which FX combination explains each of the worst 20
- [x] FirmwareFx::PARITY opt-in preset (--firmware-fx-ports parity) with a null test against the firmware model
- [x] Before/after ranked distance on the worst 20 and all 384 programs (mean 5.35 to 1.66 over 384 with PARITY, worst 20 18.14 to 2.19)
- [x] Mod 2 flipped to the default (D163, owner approval relayed 2026-11-18; `--legacy-mod2` opt-out; null test, before/after ranked distance, `render_hash` unchanged); `[ ]` rest of `PARITY` stays opt-in (owner)

## Out of Scope
Piano Touch, silent-program diagnosis, reverb ports (owner hears the firmware reverb worse), a default change for any family but Mod 2.

## Summary of Changes
- `program_rank --ablate-fx` (63 FX combinations), `--flip SETS|every-fx` (engine default with groups switched to firmware), `--jobs N`; groups moved to `nord_tools::rank_groups` (tests).
- `tools/program_rank_combo.py` (+ tests): best single / pair / exact best set / greedy / smallest explaining set, flip tables, CSVs.
- `FirmwareFx::PARITY` and `--firmware-fx-ports parity` (opt-in); `nord_tools::fx_parity` null test and `fx_parity_null` tool.
- Research note `docs/research/program-ranking-fx.md`, data CSVs, D163 (proposed flip), spec s4.0c, ROADMAP 7b.4, OQ38, audio-remaining.
- Default changed: `FirmwareFx::DEFAULT` = amp + Mod 2 (`LEGACY` = amp only, `--legacy-mod2`). 384 programs mean 5.35 to 3.78, 111 better, 15 worse (listed in the research note s3.4); `render_hash` `eb13063d...ca28` unchanged but not exercised by the hash scenario (golden FX sweep uses no raw Mod 2 words). Review fixes: GUI/plugin edits drop the raw words, `FxParams` and group copy carry them, bench scenarios and TD-18.
