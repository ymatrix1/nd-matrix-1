# Piano timbre (`Edt_Pno_kEq`, "Clav EQ"): a per-note gain, not a filter

- **Checked:** 2026-11-21
- **Sources (local, never committed):** OS v1.66 firmware (`target/fw_unpacked/bank0_CODE.bin`, `bank0_SRAM.bin`, ARM base `0xC0000000`), settled snapshot `target/keydown.snap`, the ARM harness (`tools/arm_*.py`), factory backup `.ns4b` (`Piano/Clavinet/Clavinet D6  6.1.npno` as the loaded sound). Committed: derived tables (`crates/nord-core/src/piano/timbre_gain_tables.rs`) and 108 oracle cases (`docs/research/data/piano-timbre-gain-cases.json`).
- **Question:** audio-remaining row 22 lists the Clav EQ and the LP+HP filter type as unmodelled. Which firmware routine consumes the piano `kEq` value, what does it do, and can it be ported with a check against an oracle render?

Tags follow `.agents/rules/provenance-in-code.md`. Decision D206. Earlier negative result: [amp-types-789](amp-types-789.md) s4 (the edit sends only a reload trigger; the consumer was not found).

## 1. Result

| Item | Result | Tag |
|---|---|---|
| LP+HP (synth `kFltType` 3) | Already ported: stage `0x1257B2`, opt-in `--synth-stage3`, 1840 of 1840 oracle words (D177, [synth-filter-t3](synth-filter-t3.md)). Row 22 was stale. | VERIFIED |
| `kEq` consumer | Not a DSP filter. The handler `0xC011EC2C` maps the edit value to a **timbre mode** stored at `[[core + 0x390] + 0x5C]`, then triggers a sample reload. The voice-frame builder `0xC00AA520` reads the mode of a **new** voice at `[[core + 0x390] + 0x58]`. | VERIFIED (disassembly, ARM run) |
| Modes `0x10..=0x1A` | A per-note gain, velocity and key dependent, 0.1003 dB steps, delivered as the float word `amp / 2^23` to the SMP voice record (DM word `0x9252D`, first voice). It only attenuates. | VERIFIED 108 of 108 (99 timbre cases + 9 unity) |
| Law | s3. Tables `S`, `T1`, `T2` in ROM (11 modes x 128 words), level table `G`. | VERIFIED |
| Family map (edit value to mode) | s4. | VERIFIED for the generic path; EMPIRICAL for classes 2 and 3 (field not decoded) |
| Clav class modes `0, 1, 4, 5, 8, 9, 0xC, 0xF` | Below `0x10`: no gain tables in the builder, unity. Where the Clav rocker is audible is **open**. | OPEN |

## 2. Handler and builder

- `kEq` (`0x15C`) handler `0xC011EC2C` (helper `0xC00B281C`) picks a mode by the file class byte of the loaded `.npno` (`+0x44`), stores it at `+0x5C` and sends the reload trigger. The new mode becomes `+0x58` when the sample reload completes (that promotion was not traced; the harness writes `+0x58` directly).
- Frame builder `0xC00AA520` (args: `r2` key, `r3`/`r8` internal velocity) reads the mode. With `r2 = mode - 0x10` (modes `0x10..=0x1A` only) it indexes the tables and writes the gain word. Any other mode leaves the unity word `0.25` (`0x3E800000`).
- Sounding voices are retargeted by `0xC00C07D8` when the timbre changes, with a single-velocity approximation (`S[64]`). Not modelled.

## 3. Gain law

```text
sl  = r8 >> 3                                   r8 = internal velocity, curve 6 (piano-touch-curves.md)
idx = key + 128 r2
n   = ((S[sl + 128 r2] * T1[idx] + T2[idx]) * 10) >> 16     floor, n < 0 reads 0
amp = G[n] >> 2                 n <= 60
amp = G[n - 60 k] >> (k + 2)    n > 60, k subtractions of 60
word = f32(amp / 2^23)          unity amp = 0x200000 = 0.25
```

`G[i] = round(2^23 2^(-i/60))` at `0xC02A1A08` (`G[0] = 0x7FFFFF`). Tables: `S` `0xC02A1D94`, `T1` `0xC02A3394`, `T2` `0xC02A4994`. `r8` table `0xC02B3128 + 0x18 + 2 (v + 768)`. `sl = r8 >> 3` verified for 8 of 8 velocities. Mode 0x10 (Soft on the generic path) is flat -6.02 dB at low velocity and loses less at forte on high keys.

## 4. Family map

| File class (`.npno` `+0x44`) | `kEq` to mode | Note |
|---|---|---|
| 4 (Clav) | 0..7 give `0, 1, 4, 5, 8, 9, 0xC, 0xF` | below `0x10`, unity in the builder; rocker audio OPEN |
| 2, 3 | `kEq % 6` gives `0x13..0x17` (if `[sound + 0x64B8] <= 5` only three modes) | field undecoded; five-mode table used |
| 5, 6 | `kEq & 3`: `1` gives `0x19`, `2` gives `0x11`, `3` gives `0x12` | `0x19` carries an extra -3 dB word, not modelled |
| others, generic path | `kEq & 3`: `0, 0x10, 0x11, 0x12` | run for `kEq` 0..7 on the 9 entries of 6 banks: identical rows |

## 5. Reproduce

```powershell
uv run --with unicorn --with capstone python tools/arm_piano_timbre.py capture target/fw_unpacked/bank0_CODE.bin target/fw_unpacked/bank0_SRAM.bin --cache target/keydown.snap --ns4b "<backup>.ns4b" --npno "Piano/Clavinet/Clavinet D6  6.1.npno" --out cases.json
python tools/piano_timbre_gain.py check target/fw_unpacked/bank0_CODE.bin
python tools/piano_timbre_gain.py rust target/fw_unpacked/bank0_CODE.bin > crates/nord-core/src/piano/timbre_gain_tables.rs
```

`check`: 108 cases, 0 differences. Rust test `piano::timbre_gain::tests::matches_the_arm_oracle_cases` runs the same cases (11 modes `0x10..=0x1A` and mode 0, keys 24, 60, 96, velocities 30, 80, 127).

## 6. Port

Opt-in `--firmware-piano-timbre` (`PianoLayer::firmware_timbre_gain`): the layer gain of a new note is multiplied by the timbre factor, and `process_timbre` skips the engine EQ contour while a gain mode is active. Default off; no default flip. A hardware clip of Soft / Mid / Bright against the engine contour is needed to approve a flip.

## Implications

- The "Clav EQ" of the hardware piano timbre is a velocity-dependent level tilt on the sample voice, not a filter; the engine EQ contour (`piano.rs`) is a different model. Both stay selectable.
- Row 22 shrinks to: To Rotary (closed D195), morph slots (D159, D166, D204), and the Clav-class rocker modes.

## Follow-ups

1. Clav class rocker (modes `0, 1, 4, 5, 8, 9, 0xC, 0xF`): find the consumer of the `[obj + 0x5C]` to `[obj + 0x58]` promotion at reload completion, the table `0xC0539584[core]` and the flag `0xC059868C + 0x904`.
2. Decode `[sound + 0x64B8]` (five versus three EP modes).
3. Model the voice retarget (`0xC00C07D8`), the `0x19` -3 dB word, the flag branch and the octave shift.
4. Hardware clip of Soft / Mid / Bright to decide a default.
