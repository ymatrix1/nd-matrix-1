# Piano: the program Touch velocity curves (KB TOUCH)

- **Checked:** 2026-11-18 (s4b added; s1 to s4a of 2026-11-16)
- **Sources (local, never committed):** OS v1.66 firmware (`target/fw_unpacked/bank0_CODE.bin`, `bank0_SRAM.bin`, ARM base `0xC0000000`), factory backup `Nord Stage 4 Factory Restore RevK v1.66.ns4b` (Clavinet D6 `.npno`, Felt Upright program) under `.reference/`, `docs/research/arm-edit-params.csv`, `ns4decode` (`ns4names.py`, `ns4maps.py`).
- **Question:** Which velocity curves does the per-program Touch setting (Piano `KB TOUCH`, Organ, Synth) select in the firmware, do they apply to MIDI notes, and what are the tables?

Tags follow `.agents/rules/provenance-in-code.md`. Bean `ns4-yp85`, decisions D88, D99, D117, D165 (default flip).

## 1. Result

| Item | Result | Tag |
|---|---|---|
| Which sections have a Touch | Only the Piano section: edit id `0x159` `Edt_Pno_kKbdTouch`, values 0..=2, default 1. No Organ or Synth Touch id exists in the 422-entry registry (`arm-edit-params.csv`); organ key messages carry no velocity (OQ23) and the Synth keeps its own amp velocity parameter. | VERIFIED (registry, disassembly) |
| What the value selects | Handler `0xC011AA24` stores `curveId = u32[0xC02BBF98 + 0x678 + 4 * value]` = `10, 11, 13` into field `+0x1C` of the piano layer object | VERIFIED (disassembly; run: the program recall calls the handler with index 1 for the default program and `[ctx + 0x1C]` reads 11; the table words read `10, 11, 13, 4, 4, 5`) |
| How it is used | Voice init `0xC00AA520`: `T = byte[0xC0532664 + (curveId << 10) + r8]` (`ldrb lr, [ip, r8]` at `0xC00AA654`), stroke selection index = `T`, level attenuation `X = clamp(((128 - 0x13) T) >> 7, 0, 249)` | VERIFIED (disassembly, `piano-sound-load.md` s7, s8) |
| Applies to MIDI notes | Yes: a MIDI note-on through the UART2 parser (`midi_parser` -> `midi_note_on_post`) reaches the voice init and reads the curve of the layer object. The Touch is part of the sound, not only of the keybed profile | VERIFIED (run) |
| Tables | `T(v)` for `v = 1..=127`, three curves, in `piano/touch_curve.rs` (curve 11 is `T_BY_VELOCITY`, unchanged); fingerprint pinned by a unit test | VERIFIED |
| End to end | With `[ctx + 0x1C]` set to 10, 11 and 13 and velocities 1, 2, 10, 33, 64, 90, 100, 120, 127 the byte the ARM loads equals the composed table: 27 of 27 (`tools/arm_touch_curves.py arm-check`) | VERIFIED (run) |
| Names | Edit 0 = heavy, 1 = medium, 2 = light | EMPIRICAL (monotone order of the curves, `ns4decode` `PianoTouch`) |
| Program file bits equal the edit value | File bit `1991..1992` (layer A), `+96` (layer B) = edit value: 6 of 6 in the program recall (s4); **all 384 factory programs, both layers, 384 of 384 (768 of 768 layer values) equal the firmware recall (s4b, D165)**; factory set 156 / 208 / 20 (layer A) and 176 / 206 / 2 (layer B) for 0 / 1 / 2 | VERIFIED (run) |
| Recall to level, no hand poke | A Heavy and a Light factory program recalled by the firmware reach `curveId` 10 / 13 at `[core + 0x1C]`; `T`, `X` and the Q23 amplitudes of both voices equal the Rust model: 18 of 18 velocity rows, max error 0 (s4a) | VERIFIED (run) |

## 2. Tables (derived numbers)

`r8(v) = s16[0xC02B3128 + 0x18 + 2 (v + 128 * 6)]` (`r8(1) = 17`, `r8(64) = 203`, `r8(127) = 656`; curve 6 of the 8 internal velocity curves is in force at the default program, `[manager + 0x134]`). `T = curve[curveId][r8]`.

| v | T heavy (id 10) | T medium (id 11) | T light (id 13) | X heavy / medium / light |
|---|---|---|---|---|
| 1 | 200 | 200 | 200 | 170 |
| 10 | 192 | 178 | 163 | 163 / 151 / 138 |
| 33 | 143 | 131 | 118 | 121 / 111 / 100 |
| 64 | 88 | 78 | 68 | 74 / 66 / 57 |
| 90 | 49 | 40 | 32 | 41 / 34 / 27 |
| 100 | 35 | 27 | 19 | 29 / 22 / 16 |
| 120 | 8 | 2 | 0 | 6 / 1 / 0 |
| 127 | 0 | 0 | 0 | 0 |

`X` steps of 2 per level index (0.1003 dB each, `level_q23.rs`), so at velocity 64 the light curve is 1.8 dB louder than medium and the heavy one 1.6 dB quieter; at velocity 10 the spread is +2.6 / -2.4 dB, at 120 +0.2 / -1.0 dB. `T` also moves the stroke layer (the record threshold is compared with `T`), so the timbre changes with it.

## 3. Method

1. `tools/arm_xref.py dis` of `0xC00D1FD0..0xC00D2130` (keybed event handler) and `0xC00AA520..0xC00AA6D0` (voice init) gave the table reads; `0xC011AA24..0xC011AA3C` the Touch handler. The words at `0xC02BBF98 + 0x678` read `10, 11, 13, 4, 4, 5`.
2. A run of the settled firmware (`tools/arm_keydown.py` rig, Clavinet D6 loaded with `tools/arm_piano_load.py`) with a MIDI note-on over UART2 logged `curveId = [r6 + 0x1C] = 11` and `r8 = 17 / 203 / 656` at velocity 1 / 64 / 127 for the default program.
3. A write watch on `[ctx + 0x1C]` showed the single writer `0xC011AA38` (the handler above), called by the edit dispatch with `curveId` from the table.
4. `[ctx + 0x1C]` set to each curve id, same note-on: the byte at `0xC00AA658` (`T`) equals `curve[id][r8(v)]` for 27 of 27 pairs. Reproduce: `uv run --with unicorn --with capstone python tools/arm_touch_curves.py arm-check <CODE> <SRAM> --cache target/keydown.snap --ns4b <backup> --npno "Piano/Clavinet/Clavinet D6  6.1.npno"`.
5. `python tools/arm_touch_curves.py rust <CODE>` regenerates the arrays and the fingerprint `0xc8293ca1773436e4` the Rust test pins.

## 4. Program file bits (VERIFIED by the program recall; decoder realigned by D99)

The firmware recall (`tools/arm_program_load.py` boot with Felt Upright, then `session.run(300)`, hook on the Touch handler `0xC011AA2C` and on the store setter `0xC00D051C`) with the two bits at file bit `1991` of layer A and `2087` of layer B set to 0 / 1 / 2 (layer A) and 1 / 2 / 0 (layer B): the setter value and the handler index equal the file value in all 6 of 6 cases. So the file value is the edit value: **0 heavy (curve 10), 1 medium (11), 2 light (13)**, at file bit `1991..1992` (piano payload bit `167`, layer base + 63; layer B `+96`). That is the `ns4decode` position.

Single-bit flips of the same program (layer A, file bit = `1824 + rel`) give the layer layout, setter value in brackets (layer A store `0x91050fe0`):

| rel | field | setter | value |
|---|---|---|---|
| 26 / 27 | variation | `0x155` | 2 / 1 |
| 60 / 61 | `kAcoustic` | `0x157` | 3 / 0 (default 1) |
| 62 | pedal noise | `0x158` | 0 (default 1) |
| 63..64 | touch | `0x159` | 0, 1, 2 |
| 65 | unison | `0x15d` | 2 |
| 67 / 68 | dynamic compressor | `0x15e` | 2 / 1 |
| 70 / 71 / 72 | EQ / timbre | `0x15c` | 4 / 2 / 1 |

Over the 384 factory programs the touch field reads 156 x 0, 208 x 1, 20 x 2 and never 3. Before the fix the Rust decoder (`cbin/piano_preset.rs`) read `variation` at rel 60..61, `acoustic_level` at 62..63, `pedal_noise` at 64 and the touch at 73..74 (always 0 in the factory set, so `Medium`), i.e. every factory program decoded as Medium and string resonance, soft release and pedal noise came from the wrong bits. The decoder now reads the table above; the unit tests in `cbin/piano_preset_layout.rs` pin the positions on 12 real factory blocks (3 of 4 fail on the old decoder), `ns4 preset-check` reports 0 problems on all 544 presets and 384 programs, and the lossless `to_file_like` round trip is unchanged. Which bit of `kAcoustic` is soft release is EMPIRICAL (bit 1 is set in 4 of 384 programs, bit 0 in 241).

## 4a. Program recall to level, end to end (D117)

Closes the gap D99 left: `arm-check` set `[ctx + 0x1C]` by hand. `tools/arm_program_touch.py` does not poke it. It puts the factory program into the Store record of the EEPROM image (`arm_program_load.build_image`), boots so the firmware recalls it, hooks the handler store `0xC011AA38` (`str r3, [r2, #0x1C]`) and then plays MIDI note 60 over UART2 with the program's `.npno` served through `LibraryStub` and bound by the firmware select method (`0xC00AAA58`). `r2` of the store is the piano core object of `arm_piano_load` (`[core + 0x14]` is its sound pool; `0x90c03cd0` for layer A, `0x90c32a50` for layer B in this boot). Reproduce (local firmware and backup, `C=target/fw_unpacked`):

```
python tools/arm_screens.py boot $C/bank0_CODE.bin $C/bank0_SRAM.bin --out target/oled/state.bin
python tools/arm_store.py store $C/bank0_CODE.bin $C/bank0_SRAM.bin --state target/oled/state.bin --out target/oled/eeprom.bin
python tools/arm_program_touch.py $C/bank0_CODE.bin $C/bank0_SRAM.bin --eeprom target/oled/eeprom.bin \
    --ns4b "<backup>.ns4b" --program "Program/Bank A/Pearl Upright.ns4p" \
    --npno "Piano/Upright/Pearl Upright Lrg 6.0.npno" --velocities 1 2 10 33 64 90 100 120 127
cargo run --release -p nord-tools --bin touch_model -- "<backup>.ns4b" \
    "Piano/Upright/Pearl Upright Lrg 6.0.npno" 60 0 1 2 10 33 64 90 100 120 127
```

| Program (slot) | Sound | Stored Touch | Recall `curveId` at core `+0x1C` (layer A / B) |
|---|---|---|---|
| Pearl Upright (A:11) | Pearl Upright Lrg 6.0 | 0 Heavy | 10 / 11 |
| Shimmer Upright (A:43) | Felt Upright Lrg 6.2 | 2 Light | 13 / 11 |

Per velocity, note 60 (firmware: the byte `T` at `0xC00AA658`, `x` and the Q23 amplitude of the two level calls `0xC00A1EB4`; Rust: `touch_model`, `NpnoLevel::voices_touch` with `TouchCurve`):

| v | Heavy T / X | Heavy Q23 (voice 1, voice 2) | Light T / X | Light Q23 (voice 1, voice 2) |
|---|---|---|---|---|
| 1 | 200 / 170 | 239002, 239002 | 200 / 170 | 236257, 236257 |
| 2 | 200 / 170 | 239002, 239002 | 183 / 155 | 334118, 334118 |
| 10 | 192 / 163 | 280959, 280959 | 163 / 138 | 494862, 494862 |
| 33 | 143 / 121 | 741455, 741455 | 118 / 100 | 1190662, 1190662 |
| 64 | 88 / 74 | 1743232, 1743232 | 68 / 57 | 2300208, 2049253 |
| 90 | 49 / 41 | 3000282, 3000282 | 32 / 27 | 3693780, 3693780 |
| 100 | 35 / 29 | 3526975, 3526975 | 19 / 16 | 4292341, 4342216 |
| 120 | 8 / 6 | 5408011, 5408011 | 0 / 0 | 6284355, 6212173 |
| 127 | 0 / 0 | 6284355, 6357376 | 0 / 0 | 6284355, 6212173 |

Firmware and Rust agree on `T`, `X` and both amplitudes in 9 of 9 rows per program (18 of 18, max error 0 in every column). The `x` of the level call equals `clamp(((128 - 0x13) T) >> 7, 0, 249)` in all 18 rows. So the chain program file bits, decoder, Touch handler, `curveId`, `T`, stroke, `X`, Q23 level is verified for a Heavy and a Light program with no hand-set field. The `agy` hypothesis that the recall could differ for Heavy or Light programs is refuted for these two; the other factory programs share the same bit decode (s4) and the same handler.

Factory census (`cargo run --release -p nord-tools --bin piano_touch_census -- "<backup>.ns4b"`, project decoder, 384 programs, 0 invalid): layer A reads 156 Heavy / 208 Medium / 20 Light (equal to the count of s4), layer B 176 / 206 / 2. Counting piano layers that are enabled in a section that is on: 80 Heavy / 201 Medium / 18 Light. Programs with layer A Heavy and in use: 65; Light: 18, and all 83 resolve to an `.npno` level model. Light programs: A:26 Bubbles Mk I, A:34 Stereo Wah Clav, A:42 Classic Wah Clav, A:43 Shimmer Upright, B:17 Live CP Grand, B:58 Taped Wurly, C:18 Phased Out Wurly, C:34 Vintage Wurly, C:35 Distorted Whl II, C:59 Bounce Clav, D:10 Amped Phase Clav, D:50 Rotor Clav, E:18 J3 E Pno Pad, E:27 Snappy D6, E:34, E:5, F:17, F:43 (the census prints the full Heavy and Light lists with names and slots).

Not covered: other notes and sounds than these two (same code path), layer B (stored and recalled by the same handler; both programs keep layer B on Medium, curveId 11), and everything after the Q23 amplitude (SMP bus, DAC; D26, D39).

## 4b. Every factory program recalled by the firmware (D165)

Closes the last file-bit gap: s4 flipped single bits of one program and s4a recalled two programs. `tools/arm_program_touch_bits.py` recalls **every** factory `.ns4p` (no poke, no bit flip): per program it puts the file into the Store record of the EEPROM image, boots the full machine so the firmware recalls it (about 50 s of CPU each, 14 in parallel), reads `Edt_Pno_kKbdTouch` (`0x159`) of both piano layer stores through the layer value table (`arm_edit_capture.value_cell`, the D142 method) and hooks the handler store `0xC011AA38` for `curveId`. The expected values are the decoder values of `piano_touch_census --csv` ([piano-touch-program.csv](data/piano-touch-program.csv)); the recall is committed as [piano-touch-recall.json](data/piano-touch-recall.json).

| Check | Result |
|---|---|
| Programs recalled | 384 (all `.ns4p` of the Factory Restore RevK v1.66 backup), 0 errors |
| Recalled Touch equals the decoded file bits (both layers) | **384 of 384 programs** (768 of 768 layer values: 332 Heavy, 414 Medium, 22 Light) |
| `curveId` stored by the handler equals `10 / 11 / 13` for the value | 384 of 384 programs (both layers) |
| Layer A / B combinations seen | (A, B) = (1,1) 176, (0,0) 136, (1,0) 31, (0,1) 20, (2,1) 10, (2,0) 9, (2,2) 1, (1,2) 1: every value occurs in both layers, so the layer B position `+96` is exercised too |
| Value 3 | never stored |

Reproduce (local firmware and backup, `C=target/fw_unpacked`, about 70 min on 14 workers; `check` needs no firmware and is the unit test `test_committed_recall_equals_the_decoder_census_for_every_factory_program`):

```
cargo run --release -p nord-tools --bin piano_touch_census -- "<backup>.ns4b" --csv > docs/research/data/piano-touch-program.csv
python tools/arm_program_touch_bits.py batch $C/bank0_CODE.bin $C/bank0_SRAM.bin --eeprom target/oled/eeprom.bin \
    --ns4b "<backup>.ns4b" --workers 14 --census docs/research/data/piano-touch-program.csv --out docs/research/data/piano-touch-recall.json
python tools/arm_program_touch_bits.py check docs/research/data/piano-touch-program.csv docs/research/data/piano-touch-recall.json
```

Layer client mapping (clients 2, 3 = layer A, B by order) is EMPIRICAL, as for the synth in D142: the values match per layer in all 384 programs, including the 71 programs whose two layers differ.

### Effect of the flip on the 82 factory programs with an active non-Medium Touch

`program_sweep` (new flags `--piano-touch` / `--legacy-piano-touch`; notes C3 G3 E4 C5, velocity 100, 96000 frames, default engine options, [piano-touch-sweep.csv](data/piano-touch-sweep.csv)): sections that are on, layers that are on, Heavy or Light. Whole-program RMS, Touch curves against legacy (Medium for every `.npno` note):

| Result | Value |
|---|---|
| Programs rendered, NaN / Inf / silent | 82, 0 / 0 / 0 |
| RMS change, min / median / max | -1.59 dB / -0.46 dB / +1.23 dB |
| Quieter / louder / equal | 58 / 20 / 4 (layer A Heavy: -1.59 .. +0.21 dB; layer A Light: -0.14 .. +1.23 dB; organ and synth layers of the same program dilute it) |
| Default-mode renders with clipped samples | 8 with legacy, 3 with Touch (22 and 10 of 164 renders over both modes). Total clipped samples (default mode, 82 programs) **rise 224 to 534**: Bubbles Mk I 184 to 518 and DistantGrand 2 to 14 (both Light, +0.70 dB) clip more, while six Heavy programs drop to 0 or near 0 (Astoria Grand 5 to 0, Grand Pad 5 to 0, C and G 6 to 0, Koln Grand 2 to 0, Piano Room 1 to 0, Vista Layer Whl 19 to 2). So fewer renders clip, but the two Light programs clip harder |
| Active Heavy or Light layers without a loaded `.npno` sound | none: every active Heavy or Light layer of those 82 programs resolves to a loaded `.npno` sound (sweep `sounds` fields), but the sweep does not record whether that sound carries a firmware level model: for layer A the D117 census says 83 of 83, for layer B it is unchecked, so the ASSUMED `touch_fallback_amp` is not excluded for a layer B sound |

Heavy plays quieter and Light louder, as s2 predicts (velocity 100: X 29 / 22 / 16, about -0.7 / +0.6 dB of level plus the stroke change). `render_hash` is unchanged (`eb13063d...ca28` before and after: its piano segment is Medium); no golden moves.

## 5. Implications

- **Default on since D165** (owner approval 2026-11-18): `PianoLayer::firmware_touch` / `--firmware-piano-touch` (still accepted) / GUI Firmware bar "Touch curves"; `--legacy-piano-touch` and `--legacy-all` opt out. The Touch bits of every factory program equal the firmware recall (s4b, 384 of 384) and the recall to level chain is verified for a Heavy and a Light program (s4a). Still missing, stated at the flip: no hardware clip of Heavy or Light (the clips are Medium), the sounds without a level model scale the Medium response (ASSUMED; for layer A the D117 census finds a level model in 83 of 83 Heavy or Light programs in use, layer B is unchecked, s4b), the System-menu MIDI velocity curves are not modelled. The plugin does not receive the per-program Touch (s6). Legacy (`--legacy-piano-touch`): every `.npno` note plays the Medium curve (bit identical to before) and the other sounds the ASSUMED powers.
- On (default): `.npno` sounds with a level model play the layer's curve (stroke and level); other sounds scale the Medium response by `2^(-2 (X - X_medium) / 60)` (only the Touch difference is firmware derived).
- `render_hash` and the goldens do not change (synthetic setups, Medium); a loaded factory program takes string resonance, soft release, pedal noise and Touch from their real bits, and since D165 plays its Touch.

## 6. Follow-ups

Blockers of the default flip of `firmware_touch` (D22) after D117, and their state after D165 (flipped with owner approval, 2026-11-18):

| Blocker | State | Tag |
|---|---|---|
| Program file bits to Touch to level for `.npno` sounds | closed (s4, s4a; every factory program s4b, 384 of 384) | VERIFIED |
| `touch_fallback_amp` / `TouchCurve::gain_vs_medium` for sounds without an `.npno` level model: `2^(-2 (X - X_medium) / 60)` applied to the Medium response | open, not blocking in practice: only the Touch difference is firmware derived, the Medium response of those sounds is the ASSUMED power curve; layer A: all 83 Heavy or Light programs in use resolve to a level model (D117); layer B unchecked | ASSUMED |
| System-menu MIDI velocity curves (`Edt_Sys_kMIDIVelCurve`, `Edt_Ext_kVelCurve`, `Edt_Com_kAuxKbdVelCurve`) choose the `r8` table before `T` | open, not modelled (item 1 below); s4a ran with the default `r8` curve 6 | ASSUMED (default curve) |
| Hardware clips for Heavy and Light (all existing clips are Medium) | open, accepted by the owner at the flip: the firmware numbers are exact, whether the product matches the model after the DAC for those curves is unmeasured | missing evidence |
| Re-baseline `render_hash` and the goldens | not needed: `render_hash` identical before and after (s4b) | VERIFIED (run) |

1. The MIDI velocity curve of the System menu (`Edt_Sys_kMIDIVelCurve`, `Edt_Ext_kVelCurve`, `Edt_Com_kAuxKbdVelCurve`: three values each) selects the other velocity table (`r8` curves 1, 2 and the default 6, `0xC00D2024`); not modelled.
2. Compare a Light and a Heavy program against hardware clips once such clips exist; if the measured level disagrees, `--legacy-piano-touch` is the fallback.
3. The plugin (`nord-plugin`, `Params::sync_to_engine`, `params.rs`) writes the global `piano_touch` parameter (default Medium) to both layers on every sync, so a program's own per-layer Touch never reaches the plugin engine: the default flip changes the standalone and GUI paths only. Wiring the program Touch into the plugin needs the program load to set the parameter (or the sync to skip it); not done.
4. Model the System-menu MIDI velocity curves (item 1) and check that every non-Medium layer's sound carries a firmware level model (layer B; layer A is 83 of 83, D117).
