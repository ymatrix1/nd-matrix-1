# Firmware-derived data inventory (technical, not legal)

- **Checked:** 2026-11-17, against `origin/main` @ `be206e39`.
- **Sources:** `git ls-files`, `git grep` for `include_str!`, `include_bytes!`, `VERIFIED` tags and "derived" doc comments in `crates/`; sizes from the working tree; container layout from [ns4-firmware-teardown.md](ns4-firmware-teardown.md); runtime precedent `crates/nord-core/src/synth/fw_wave_tables.rs` (D22, D127). This is a technical inventory of what is tracked. It draws no legal conclusion.
- **Question:** Which tracked files or embedded tables are copied or extracted from the Nord firmware or other Nord content, and which of them can be read at runtime from the user's own OS update file instead?

## Classes

| Class | Meaning | Policy |
|---|---|---|
| A | Raw artifact: installer, binary dump, disassembly | Never tracked. Removed here. |
| B | Code image: DM/PM words or instruction rows of a firmware routine, replayed by a Rust port | Move to runtime load, test with a synthetic image |
| C | Large numeric table dumped word for word from firmware memory | Move to runtime load where large, test with a synthetic table |
| D | Measurement of our own probes, message logs, goldens, fitted laws | Stays. Review per file that no raw firmware bytes are inside |

[D20](../DECISIONS.md#d20-derived-values-only-no-firmware-bytes-in-the-repository) already says "derived values only; a table the code needs is committed as a closed form". Classes B and C contain tables and images that have no closed form, so they go beyond D20 as written. D128 closes that gap.

## Class A: removed in this PR

| Path | Size | What | Users |
|---|---|---|---|
| `helper.exe` | 587,736 B | NSIS USB driver installer, resource 105 of the Nord OS updater | none (`git grep helper.exe`: no hit outside the file) |
| `sram.bin` | 64,640 B | Same size as the firmware `SRAM` segment (`0xFC80`) | none. `tools/tests/test_ns4_param_curves.py` builds its own `tmp_path / "sram.bin"` |
| `sram.asm` | ~420 KB | Disassembly of `sram.bin` | none |

Added in early research commits `d2a59e70` and `ede626f2`. Docs mention `SRAM/core1_sram.bin`, `core2_sram.bin`: those are local, git-ignored files under `SRAM/` or `target/`, not these. `.gitignore` now blocks the three root names and `*.exe`, `*.dmg`, `*.ns4b`, `*_sram.bin`, `*_code.bin`. No tracked path matched any new pattern (`git ls-files` check). `.reference/`, `target/`, `.venv/` were already ignored.

**History still contains the removed files.** `git rm` does not delete them from earlier commits, forks or clones. Rewriting history (filter-repo, force push) is the owner's decision. This PR does not do it.

## Class B: code images

| Path | Size | What | Used by | Runtime source | Replacement fixture |
|---|---|---|---|---|---|
| `crates/nord-core/src/organ/vox_image.rs` | 4 KB | 84 start words, sine phases, 36 ladder words | Vox post stage | PNO image, `tools/organ_vox_image.py` | Synthetic words |
| `crates/nord-core/src/organ/firmware_bank.rs` | 19 KB | tonewheel bank constants and table notes | organ tonewheel bank | PNO image (`tools/organ_oracle_audio.py table`) | Synthetic table |

**Migrated: Farfisa processor (2026-11-23, D128 update).** `farfisa-fw-image.csv`, `farfisa_fw/prog_proc.rs` and, found while doing it, `b3_bass_fw/native_farfisa.rs` (29 KB of Rust generated from the same rows, not listed above) are removed. `organ/farfisa_fw/image.rs` builds the image from the user's update file (`NS4_WAVE_TABLES`), `rows.rs` decodes the rows, `crate::dsp_image` finds the PNO image and lays out core 1; tests use a synthetic image. **Migrated: B3 Bass chain (2026-11-24, D128 update).** `b3_bass_fw/prog_gen.rs`, `prog_proc.rs`, `native_bass_gen.rs`, `native_bass_proc.rs` (generated straight line translations of the same rows), `native_gen.rs` (their generator, a class D name false positive) and `bass-fw-image.csv` are removed. `organ/b3_bass_fw/image.rs` builds the 555 and 898 rows and the 6,881 word DM image from the user's update file; the shared row decoder is now `organ/sharc_rows/`, the DM text builder `dsp_image::dm_text`. Of the 6,881 words 2,156 are loader data; 4,732 differ from it: ARM boot and install output. Of those, 2,834 are words the ARM copies from its own constants, which `bank0_CODE` of the same file holds, so the delta names them by byte offset (`A,addr,count,offset`) and carries no value; 1,898 (7 of them clears) are inline in `bass-fw-install-delta.csv`. The delta is **class D by the Farfisa precedent (our capture of ARM output), flagged for owner review**: at 1,898 inline words it is not small, and the owner may prefer to hold it locally.

Runtime path for all of B: the user's OS update `.exe` carries the firmware as PE resource 101, a container with segments `SRAM` (`0xFC80` B), `CODE` (`0x53EA70` B) and `ARM0`. `FirmwareContainer::extract_segment(bank, name, raw)` in `crates/nord-core/src/firmware/mod.rs` returns the decompressed segment. The DSP images (core 1 and core 2 SRAM) are inside `CODE`; the tools under `tools/` already locate them. Open point: the mapping from container segment to per-core DSP image has to be written once as a loader (research [dsp-image-hunt.md](dsp-image-hunt.md)).

## Class C: numeric tables

Large tables (above ~20 KB) first, they have the highest size and the clearest runtime source.

| Path | Size | What | Used by | Runtime source | Replacement fixture |
|---|---|---|---|---|---|
| `docs/research/data/synth-p-words.csv` | 370 KB | per-type P block words (`DM` addresses, float words) | `synth/p_words.rs` | SY/ORG core 2 DM | 3-row CSV |
| `docs/research/data/fx-type-words.csv` | 133 KB | FX type word lists (slot, type, dm_word) | FX type tests, tools | FX cores DM | small CSV |
| `docs/research/data/synth-super24-tables.txt` | 74 KB | 3072 rows of Super24 table words | `synth/fw_super2.rs` | core 2 DM | synthetic 16 rows |
| `docs/research/data/synth-fm-tables.csv` | 56 KB | FM tables (`type, dm, word`) | `synth/fw_fm/table.rs` | core 2 DM | small CSV |
| `docs/research/data/synth-ladder-coef-table.csv` + `synth-ladder-shaper-table.csv` | 6 KB + 17 KB | Ladder24 coefficient rows and shaper | `synth/fw_ladder_tables.rs` (generated, 21 KB) | core 2 DM `0xC02DE` | closed-form stand-in |
| `docs/research/data/fx-mod2-tables.csv` | 26 KB | `ENS` (1025 words), `PHAS` (258 words) | `fx/fw_mod2_tables.rs` (18 KB) | FX core 2 DM `0xC0BD4`, `0xE3D81` | short ramp |
| `crates/nord-core/src/fx/fw_drive_tables.rs` | 19 KB | three Amp drive tables, float words from FX core 1 `0xE0000..` | `fx/fw_drive_tables` | FX core 1 DM | synthetic tables |
| `crates/nord-core/src/fx/fw_mod2_dense.rs` | 27 KB | dense Mod 2 laws, ARM capture knots | Mod 2 | ARM capture (class D by origin, see below) | knots |
| `docs/research/arm-chan-tables.csv` | 65 KB | ARM channel tables (`table,index,target,base,size,tag`) | tools, docs | ARM0 image | small CSV |
| `docs/research/arm-param-table.csv` | 34 KB | ARM parameter table | `tools/`, param inventory | ARM0 image | small CSV |
| `docs/research/data/oled-strings-vs-firmware.csv` | 62 KB | OLED strings compared with firmware strings | GUI OLED text, [oled-strings-vs-firmware.md](oled-strings-vs-firmware.md) | ARM0 string table | key list only |
| `crates/nord-core/src/synth/p_table.rs` | 20 KB | per-wave P table (EMPIRICAL fit, not a dump) | `synth/p_table.rs` | not applicable, fitted to hardware clips | stays (class D) |
| `crates/nord-core/src/fx/fw_delay_table.rs`, `fw_eq_tables.rs`, `fw_coeffs.rs`, `fx/tables.rs` | 11, 7, 9, 9 KB | delay thresholds, EQ words, biquad sets | FX | FX / ARM images | synthetic |
| `docs/research/data/synth-filter-tables.csv`, `synth-filter-table-d.csv`, `synth-env-rate-table.csv`, `synth-env-rates-settled.csv`, `synth-sync-table.json`, `synth-knob-laws.csv`, `synth-velocity-law.csv`, `synth-wave-types.csv` | 2 to 93 KB | small DM tables and laws | `synth/fw_*.rs` | core 2 DM or ARM | synthetic |
| `docs/research/data/synth-filter-t3-tables.csv` + `crates/nord-core/src/synth/fw_stage3_tables.rs` | 5 KB + 6 KB | `T0`, `T1`, `T2` words of stage `0x1257B2` (129 each) | `synth/fw_stage3_tables.rs` (generated) | core 2 DM `0xB0A9F`, `0xB0C0D`, `0xB0C8E` | closed-form stand-in |
| `crates/nord-core/src/synth/fw_filter_tables.rs` | 7 KB | resonance tables A, B, C (129 words each, generated from `synth-filter-tables.csv`) | synth filter | core 2 DM `0xB0D0F`, `0xB0D90`, `0xB1015` | synthetic tables |
| `crates/nord-core/src/fx/fw_lp24_tables.rs` | 7 KB | Amp LP24 words captured from the ARM (generated from `fx-lp24-words.csv`) | `fx/fw_lp24` | FX core 1 context `0xC1A28` | synthetic tables |
| `crates/nord-core/src/engine/zone_table.rs` | 14 KB | key ranges of zone codes | engine | ARM0 code constants | closed form from spec |

Already at runtime (the model to copy): `synth/fw_wave_tables.rs` reads the 46 WAVE tables from the user's image (`--synth-wave-tables PATH`, `NS4_WAVE_TABLES`; D22, D127). Its registry CSV `synth-wave-registry.csv` holds addresses and counts only.

## Class D: measurements, logs, goldens, fitted laws

| Path | Size | What | Used by | Note |
|---|---|---|---|---|
| `docs/research/data/*.script`, `*.msg` (arm-replay-*, keydown-*, org-system-*, pno-pass-*) | 0.3 to 455 KB each | message logs and DSP install scripts from our own ARM/oracle runs | `sharc-*` replay tests and tools | Contain message words and addresses sent by the firmware. Largest: `org-system-load-pno1.script` 455 KB. Review whether the install scripts embed image words. |
| `docs/research/data/farfisa-fw-install-delta.csv` | 1 KB | 46 DM words, 2 clears and 10 start registers of the Farfisa model 2 install, captured from our ARM install run (`tools/organ_farfisa_fw.py image`) | `organ/farfisa_fw/image.rs` | Replaces the 802 word image; 756 of those words are now read from the user's file. Owner review wanted: the 46 words are ARM output (float coefficients), kept as our own measurement |
| `docs/research/data/bass-fw-install-delta.csv` | 17 KB | 12 start registers, 1,898 inline DM words (7 clears) and 53 `A` lines (2,834 words named by byte offset into the user's `bank0_CODE`, no values) of the B3 Bass boot and model 5 install, captured from our ARM install run (`tools/organ_bass_fw_gen.py image`, `convert`) | `organ/b3_bass_fw/image.rs` | Replaces the 6,881 word image; 2,156 of those words are read from the user's file, 2,834 more are named by offset. Owner review wanted: the inline words are ARM output (float and integer coefficients), kept as our own measurement; they could also be held locally. Another OS version fails the layout guard (the offsets move, the rows do not) Provenance of the 1,891 non zero inline words (checked 2026-11-24 against the unpacked v1.66 segments): 466 are small integers (below 0x10000) and 380 look like DM addresses; 1,045 are 32 bit patterns of which only 64 occur as an aligned word anywhere in the update file, and about 981 (969 read as plausible float32) occur nowhere in it. They are ARM install output (state the ARM writes into DSP DM at boot and model 5 install, captured with `sharc-chain`), not bytes copied from the file; they were not computed by our code. Class D, owner review. |
| `docs/research/data/bass-*-golden*.csv`, `farfisa-*-golden*.csv`, `*-chord-goldens.csv` | 1 to 43 KB | our oracle output used as test goldens | organ tests | Our own runs, stay |
| `docs/research/data/keydown-*.csv`, `bass-keydown.csv`, `bass-state.csv`, `bass-voice-slots.csv` | up to 231 KB | oracle observations of key-down states | organ tests | stay |
| `docs/research/data/synth-*-cases.json`, `synth-stage-oracle-cases*.json` | up to 166 KB | oracle input/output cases | synth stage tests | stay |
| `docs/research/data/fx-*` (rows, dense, fits, reverb, small-id laws) | 1 to 158 KB | ARM harness captures and fitted laws | FX tests | stay; `fx-param-fits.csv` is a capture of firmware words, class D by origin |
| `docs/research/data/arm-capture-*`, `arm-dsp-capture-*`, `arm-edit-capture-*`, `arm-bulk-tables.json` | up to 250 KB | ARM to DSP capture of the firmware's own recall | `tools/arm_dsp_*` | stay |
| `docs/research/data/piano_events_*.json`, `piano_flags_*.csv`, `emu_vs_hw_*.json`, `organ_k_*.json`, `muscriptor_agreement.json` | up to 1.2 MB | transcriptions and comparisons against third-party video recordings | evaluation tools | Derived from recordings, not from the firmware. Keep, but the recordings are Nord content; note in follow-up 6. |
| `docs/research/data/program-sweep.json`, `synth-program-velocity.csv`, `synth-sustped-programs.csv` | up to 208 KB | per factory program results (program names and parameters) | `program_sweep` | Names of factory programs are Nord content; values are our output |
| `docs/research/sharc-fixed/*.csv` | 312 KB, 999 KB | fixed-point probe results of our own SHARC runs | docs | stay |
| `crates/nord-panel/data/photo_anchors_*.csv`, `assets/panel/proof/*.png`, `assets/blender/build_proof.py` | small, PNG up to 1 MB | panel geometry anchors and proof renders of the panel look | `nord-panel` | Not firmware. Source photos are Nord product images: confirm the provenance in follow-up 6 |

## Enforcement

`python tools/check_fw_derived.py` (CI step "Firmware-derived files only shrink (ratchet)", tests in `tools/tests/test_check_fw_derived.py`) reads `tools/fw-derived-baseline.txt`, which lists every tracked class B and C path (2 B, 31 C) plus 2 reviewed class D false positives of the name heuristics (`fw_wave_tables.rs`, `arm-bulk-tables.json`). It fails when:

- a baseline path is no longer tracked (stale: remove the line, the ratchet only goes down);
- a new tracked file looks derived and is not in the baseline: class A patterns, `*-fw-image.*`, `fw_*_tables.rs`, `prog_*.rs` or `native_*.rs` under `*_fw/`, or names containing `fw-image`, `-tables.`, `p-words` under `docs/research/data/` or `crates/`;
- a tracked binary over 32 KB (png, jpg, bin, ...) is outside the allowlist in the script (GUI panel art and screenshots only).

Review of the lists above against `git ls-files` (2026-11-23) found four tracked class C files the tables did not name; they are now listed under Class C and in the baseline: `docs/research/data/synth-filter-t3-tables.csv`, `crates/nord-core/src/synth/fw_stage3_tables.rs`, `crates/nord-core/src/synth/fw_filter_tables.rs`, `crates/nord-core/src/fx/fw_lp24_tables.rs`. `synth/p_table.rs` is named in the Class C table but marked "stays (class D)", so it is not in the baseline. `fw_wave_tables.rs` (runtime loader) and `arm-bulk-tables.json` (own harness capture) match the name heuristics and are baselined as class D.

## Counts

- `VERIFIED` tags appear in 355 files under `crates/`; most state a check against a local image, not a copy. This inventory lists only files that hold the copied values.
- `.reference/`, `target/fw_unpacked/` and `SRAM/` hold the local sources; none is tracked.

## Implications

- After this PR no raw firmware artifact is tracked. Derived data (B, C) is still tracked and exceeds the D20 wording for tables without a closed form.
- The runtime loader pattern exists (WAVE tables). B and C can follow it: find the segment with `extract_segment`, read the words at the documented DM addresses, check a short fingerprint, and fall back to the existing behaviour (silence or the legacy model) when no image is given.
- Default renders (`render_hash`) must not change silently: moving a table to runtime loading changes the default for users without an image. Each migration needs an explicit fallback and a note in D22 terms.

## Follow-ups (ordered by legal risk first, then audio impact / effort)

1. **Organ code images — Class B first** (`vox_image.rs`, `firmware_bank.rs`; the Farfisa and Bass files are done, see above, and `crate::dsp_image` is the shared loader for the rest): literal firmware translations and instruction rows carry the highest copyright risk, and organ firmware ports are opt-in (D22), so migrating them first removes the largest legal exposure with zero default-audio breakage.
2. **Synth P words, Super24, FM, Ladder24 tables** (`synth-p-words.csv`, `synth-super24-tables.txt`, `synth-fm-tables.csv`, `fw_ladder_tables.rs`): the Synth is on the default audio path, the files are the largest (about 570 KB), and the WAVE loader gives the template. High impact, medium effort. Start with `synth-p-words.csv` (370 KB, one reader `p_words.rs`).
3. **FX tables** (`fw_drive_tables.rs`, `fw_mod2_tables.rs`, `fw_delay_table.rs`, `fw_eq_tables.rs`, `fw_coeffs.rs`): the Amp is the default for Twin, JC and Small (D22), so impact is high; each table is small and has a fixed DM address, so effort is low per table. Most tables have an `fx_mod2_tables.py check` style comparison script to reuse.
4. **ARM tables** (`arm-chan-tables.csv`, `arm-param-table.csv`, `oled-strings-vs-firmware.csv`, `zone_table.rs`): used by tools and GUI text, little audio impact. Low effort for the CSVs (move to local `target/` output, keep the generator), medium for OLED strings (GUI needs a fallback text).
5. **Message logs and install scripts** (class D, `org-system-load-*.script`, `pno-pass-install-*.script`): confirm they hold message words only and no image words; if not, regenerate from the local image in CI-less tests and keep a short synthetic script.
6. **Provenance of third-party content in class D** (video transcriptions, panel photos, factory program names): list the origin of each in its data README. No code change.
7. **History**: removed files remain in earlier commits. Whether to rewrite history is the owner's decision; nothing in this inventory or PR does it.
