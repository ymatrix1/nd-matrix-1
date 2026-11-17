# Research: what gates the B3 Leslie `0x1256AD`? (not found in the paths run; stage measured directly in the oracle port; D148)

- **Checked:** 2026-11-17
- **Sources (local, never committed):** OS v1.66 ARM image `target/fw_unpacked/bank0_CODE.bin` and the settled ARM harness (`tools/arm_table_watch.py`, new edit ids and a 6,000-tick recall of `B3 Chords n Solo.ns4p`), the `0x1256AD` port in `crates/nord-core/src/fx/fw_leslie*.rs` (word for word with the `sharc-fx` oracle, goldens in `tests_fw_leslie_golden.rs`), the Part A demo video segment `organ_b3` (25 s from 238 s, cut with `ffmpeg`, audio never committed) and the native renders of `tools/emu_vs_hw.py`. New: `crates/nord-tools/src/bin/leslie_probe.rs` (+ `leslie_probe.rs` helpers). Numbers and addresses only; no firmware, sample or audio content is printed or committed. Builds on [leslie-locate](leslie-locate.md) (D139), [b3-leslie-fx-chip](b3-leslie-fx-chip.md) (D135), [b3-leslie-pno-lane](b3-leslie-pno-lane.md) (D126), [fx-leslie](fx-leslie.md), [leslie-settings](leslie-settings.md), [fx-type-map](fx-type-map.md), [audio-remaining](audio-remaining.md) row 9.
- **Question:** What enables the B3 Leslie stage `0x1256AD` in the firmware (rotary speed / enable bits, organ vs piano layer, mode flags), which chip and core runs it, and, with the stage run directly in the oracle port, what are its rotor rates, modulation depths and horn / drum split against the engine rotary and the hardware clip?

Tags follow `.agents/rules/provenance-in-code.md`: **VERIFIED** = reproduced by a command under Reproduce or a unit test, **EMPIRICAL** = measured on this host (one clip), **ASSUMED** = reading without a source.

## 1. Short answer

**The gate was not found in the paths run** (PNO core 1 words, 120-tick edits and one 6,000-tick recall; FX cores and System routing ids not tried). Three new negative results narrow it, one positive one proves the hardware clip contains a rotary, and the stage itself is measured. No code path of the engine changes.

| # | Check | Result | Tag |
|---|---|---|---|
| 1 | Does the amp type edit (`Edt_Effects_kAmpTypeNew`, id `0x1a7`) with 7 "To Rotary" install the Leslie? | **No.** Value 7, 0 and 7 again on the organ FX unit (units 3 and 9) after an 800-tick recall: the three edits send the same 8 PNO core 1 words (`DM(0xB0407, 0xB0406, 0xB03FF, 0xB0476, 0xB0479, 0xB0477, 0xB047A, 0xB047B)`; ARM writers `0xC01F5BB0..0xC01F5C30`, `0xC021CC18..0xC021CD74`) plus the three rotor speed words. Types 0 and 7 are word for word equal; no `0x1256AD`, no chain head `0xC2F48..` write; row 6 of table `0xC05450C8` has **0 accesses** in the three 120-tick runs. So "To Rotary" is not a module install. | VERIFIED by a local run (needs the local firmware, not reproducible from the repo alone; counts: 3 edits, 8 identical words for 7 and 0, 0 row 6 accesses) |
| 2 | What are the stage sets `DM(0xB0476..0xB047D / 0xB047E..0xB0485)` that D139 read as "Leslie family"? | They are the **Amp module** words: the amp type edit (7 and 0 alike) rewrites exactly the first set (`0x1250CF`, `0x125371`, `0x121BBA`) and the recall installs both at tick 178 (D139 row 4). [fx-type-map](fx-type-map.md) s8 to s10 already measured `0x125371` as the Twin drive stage and `0x1235AA` / `0x1236F8` as the cabinet. They are not Leslie installs and need no gate hunt. | VERIFIED by the same local run (needs local firmware; the same 8 words, 0 accesses); the reading as Amp module follows [fx-type-map](fx-type-map.md) s8 to s10 |
| 3 | Longer recall: does the ARM ever read row 6 after the boot? | **No.** A 6,000-tick recall of `B3 Chords n Solo` (7.5 times the 800 ticks of D139; 0.25 s of ARM time at 24,000 ticks per second) has **0 accesses** to `0xC0545128..0xC0545148` after the boot (the boot has the same resolver reads as in D139, positive control). | VERIFIED by a local run (needs local firmware; counts: 8 boot accesses, 0 accesses in 6,000 ticks) |
| 4 | Does the hardware clip contain a rotary at all? | **Yes.** `organ_b3` (25 s): envelope spectrum of the 800 to 1,600 Hz band has its strongest component at **0.773 Hz** (depth 0.55) and the 100 to 700 Hz band at **7.000 Hz** (depth 0.23), with 6.974 Hz (depth 0.10) in the horn band. These are the horn slow and fast rates (0.80 and 7.0 Hz nominal, physical 0.787 and 6.889 Hz). The clip is therefore not a Leslie-off program, and a Leslie stage does run on the hardware. | EMPIRICAL (one clip, chord onsets also modulate the envelope) |
| 5a | Is the probed stage the oracle's? | `FwLeslieE40` equals the `sharc-fx` oracle word for word on the golden vectors of `tests_fw_leslie_golden.rs` | VERIFIED (unit tests) |
| 5b | Stage measured with sines (section 2, [data/leslie-probe.txt](data/leslie-probe.txt)) | Rates follow the speed words; gain about -4.3 dB at 110 Hz, +6.4 dB at 2 kHz, -11.6 dB at 5 kHz; AM depth 0.33 to 0.69 at 110 to 440 Hz and 0.70 to 0.83 from 550 Hz at slow speed (fast speed falls to 0.59 at 5 kHz and 0.51 at 8 kHz); strongest component at the **horn** rate in the left-lane sines | EMPIRICAL (one level, ring model `ADVANCE`) |
| 6 | Parameter gaps against the engine rotary | Crossover and rate scale, see section 4. No gap is clear enough to port | EMPIRICAL |

## 2. The stage in the oracle port

`leslie_probe` drives `FwLeslieE40` (ring `ADVANCE`, the clip-favoured model, D63 / D94) with a sine in the left lane (the organ bus layout) at 43,402.34 Hz for 8 s (1 s skipped) and reads the two output lanes. Speed words come from the ARM law `int(hz * 8388607 / 11025) << 8` with nominal slow 0.73 (rotor) / 0.80 (horn) and fast 6.4 / 7.0 Hz (`0x22B00 / 0x26000` slow, VERIFIED). Physical rates are nominal x `F / 44,100` = x 0.98418: slow horn 0.787, slow rotor 0.718, fast horn 6.889, fast rotor 6.299 Hz. Gain is output RMS over input RMS, depth is `(p95 - p5) / (p95 + p5)` of the 2.95 ms block RMS, "rate" the strongest envelope component in 0.3 to 20 Hz. Lane 0 shown (lane 1 within 0.5 dB and 0.07 depth except where noted).

| Sine Hz | Gain dB | AM depth slow | AM depth fast | Strongest rate fast (Hz) | Depth at drum rate 6.30 / at horn rate 6.89 (fast) |
|---:|---:|---:|---:|---:|---|
| 110 | -4.3 | 0.35 | 0.33 | 6.90 | 0.057 / 0.156 |
| 220 | -5.1 | 0.59 | 0.60 | 6.90 | 0.071 / 0.523 |
| 330 | -7.1 | 0.69 | 0.68 | 13.80 | 0.122 / 0.122 |
| 440 | -1.6 | 0.39 | 0.40 | 6.90 | 0.042 / 0.237 |
| 550 | -3.6 | 0.81 | 0.83 | 6.90 | 0.017 / 0.622 |
| 660 | -7.2 | 0.70 | 0.72 | 6.90 | 0.045 / 0.330 |
| 880 | -0.8 | 0.76 | 0.76 | 13.80 | 0.015 / 0.285 |
| 1,320 | +1.9 | 0.77 | 0.75 | 6.90 | 0.057 / 0.276 |
| 2,000 | +6.4 | 0.75 | 0.72 | 6.90 | 0.071 / 0.209 |
| 3,000 | +3.4 | 0.77 | 0.70 | 13.80 | 0.007 / 0.177 |
| 5,000 | -11.6 | 0.75 | 0.59 | 6.90 | 0.019 / 0.170 |
| 8,000 | -13.7 | 0.75 | 0.51 | 8.35 | 0.015 / 0.100 |

Readings (EMPIRICAL, one signal level 0.3 peak, ring model `ADVANCE`; the real ring is a DMA the oracle does not run, D63):

- **Rotor rates:** the strongest envelope component at fast speed is 6.90 Hz at 8 of the 12 listed frequencies (13.80 Hz, its second harmonic, at 3; 8.35 Hz at 8 kHz), against the predicted horn 6.889 Hz (grid 0.05 Hz). At slow speed the horn (0.787) and drum (0.718) lines are 0.07 Hz apart, inside the analysis resolution (8 s window), so they are not separable (strongest 0.75 to 0.80 Hz at 110, 220, 550, 660, 1,100 Hz).
- **Which word drives the envelope:** a mixed run (rotor word fast 6.4, horn word slow 0.80) gives 0.80 Hz at 110, 220, 550, 1,100 Hz and a depth at the 6.30 Hz drum line of 0.015 to 0.07 (0.128 at 330 Hz); the reverse run (rotor slow, horn fast) gives 6.90 Hz. The amplitude envelope of every tested band follows the **horn word `0xB2F26`**; the rotor word `0xB2F25` shows no amplitude line (depth at its rate 0.007 to 0.12, the level of the other off-lines). The drum is therefore either a phase / Doppler modulation with no AM (not measured here) or its amplitude line is hidden by the cabinet ripple. ASSUMED, not shown.
- **Horn / drum crossover:** the AM depth is 0.33 to 0.69 at 110 to 440 Hz (0.69 at 330 Hz, dip 0.39 at 440 Hz) and 0.70 to 0.83 from 550 Hz up at slow speed (at fast speed 0.59 at 5 kHz and 0.51 at 8 kHz) and the gain ripple (-1.6 dB at 440, -7.2 at 660, -0.8 at 880 Hz, a cabinet comb). A crossover between roughly 330 and 550 Hz is consistent, but the ripple means a sine cannot give one number; the engine uses 800 Hz (`ROTARY_CROSSOVER_HZ`, ASSUMED).
- **Spectral shape:** +6.4 dB at 2 kHz, +3.4 dB at 3 kHz, -7.8 dB at 4 kHz, -11.6 dB at 5 kHz and -13.7 dB at 8 kHz: a strong presence bump and a steep high cut, which is why the engine rotary (flat crossover, `bass_gain = horn_gain = 1`) is 13 to 19 dB too low in presence and brilliance on the clip (D63 table) and the firmware stage moves them to -5.7 and +2.3 dB.
- **Impulse** (left lane in, rotor already running, 4,096 frames): slow speed peak 0.0976 at frame 188 (lane 0) and 0.0971 at 200 (lane 1), energy 0.265 / 0.268; fast speed 0.0976 at 200 and 0.129 at 35, energy 0.233 / 0.352. The rotor-fast / horn-slow mix gives the slow numbers (188, 200) and the reverse the fast ones (200, 35), so the impulse delays also follow the horn word, not the rotor word.

## 3. Gate search (negative)

- **Chain-element constructor `0xC0227D60`** (the object that builds the chain heads of table `0xC0546F28` rows 20 to 23, D126 follow-up): three call sites, `0xC0115700` (the FX part unit builder; `r1`, `r2`, `r3` and the stack byte are loaded from per-part byte and word arrays at `fp + 0xc4 / 0xd0 / 0x100 / 0x10c`), and `0xC0118368` / `0xC011838C` (two fixed objects, `r1 = 8 / 0x12`, `r2 = 0x38 / 0x39`). All three are boot-time object builders; none takes a Leslie row as input, so the heads are not written by a Leslie specific constructor. VERIFIED (`tools/arm_xref.py callers`, `dis` listing), reading of the roles ASSUMED.
- **Row 6 reader:** the channel resolver `0xC0220D60..0xC0220E3C` walks the list `[this + 0xC8]`, takes each node's descriptor and index (`[node + 4]`, `[node + 8]`), calls the resolver `0xC0013928` and stores the resolved row through a virtual setter. It binds rows to objects; it sends nothing. So row 6 is a binding, not a send. VERIFIED (listing).
- **Edits:** enable (`0x189`), drive (`0x18a`), speed (`0x18d`) in D139, amp type 7 here: none sends `0x1256AD`. Not tried: the System menu routing ids `Edt_Sys_kAudioRoutingDstLeslie` (`0x4e`, 0..8, default 8) and `Edt_Com_kAudioRoutingBusLeslie` (`0x59`, 0..5, default 5), `Edt_Sys_kLeslieBalance` (`0x96`) and `Edt_OrgCom_kLeslieStopPreset` (`0xdf`); the harness helper only drives the organ FX unit store, the System ids need the system store. **The presence of a Leslie output bus and an output routing destination suggests the rotary is a routing target (an audio bus), not a chain element** (ASSUMED).
- **6,000-tick recall:** 0 reads or writes of row 6 after the boot (row 3 of section 1). The row is not touched by the ARM tick tasks in 250 ms of ARM time (6,000 ticks at 24,000 per second).

## 4. Comparison with the engine rotary (numbers)

| Item | Firmware stage (oracle port) | Engine rotary (`organ/rotary.rs`) | Difference |
|---|---|---|---|
| Rotor / horn rates | ARM nominal x 0.98418; slow horn 0.787, fast horn 6.889 Hz | same (`rotor_arm`, D63) | none by construction |
| Rate on the hardware clip | horn lines 0.773 and 6.974 to 7.000 Hz | model lines 0.774 to 0.906 and 6.82 to 6.92 Hz (render of the same clip, same analysis) | clip fast line 7.00 against 6.89 (+1.6 %, 0.11 Hz, 3 analysis bins of 0.04 Hz); slow line within 1 bin. One clip, a speed ramp in the window is not excluded |
| Crossover | consistent with 330 to 550 Hz; gain ripple of +/-3 dB between 330 and 880 Hz | 800 Hz one pole, flat | engine crossover about 250 to 470 Hz high (ASSUMED) |
| Gain shape | -4.3 dB (110 Hz), +6.4 dB (2 kHz), -11.6 dB (5 kHz) | flat, `tanh` drive | presence and brilliance: engine -19.2 and -12.8 dB, firmware -5.7 and +2.3 dB on the clip |
| Envelope depth of the 100 to 700 Hz band at its 6.8 to 7.0 Hz line, render of the whole clip (not the probe) | 0.155 (`--leslie firmware --leslie-ring advance`) | 0.253 (`--rotary-drive scaled`) | clip 0.230: engine +0.8 dB, firmware -3.4 dB from the clip |
| AM depth from 550 Hz up at slow speed | 0.70 to 0.83 | engine rotary full depth on the treble band | not separated here |

Hardware clip band deltas are in [audio-remaining](audio-remaining.md) row 9 and [fx-leslie](fx-leslie.md) s11.4; they are not re-measured (the render numbers above reproduce D63: score 3.42 firmware against 7.74 engine rotary on the same clip, `tools/emu_vs_hw.py`).

**Port or fix:** none. The only hardware-facing discrepancy, the fast line 7.00 against 6.89 Hz, comes from one clip whose speed history is unknown; it needs a second clip with a steady fast speed (or the pedal / switch toggled early) before the physical ratio `F / 44,100` of D63 can be called wrong, and the cheap opt-in (a rate ratio of 1.0 in `rotor_arm::physical_ratio`) would be fitted to one number.

## 5. Implications

1. A rotary is present in the hardware clip (envelope lines at the horn rates). That does not prove that the `0x1256AD` stage is the one running; the open item is how the firmware links it, if it does.
2. In the paths run, amp type 7 "To Rotary" does not install it: it writes the same PNO core 1 words as type 0. This partly answers follow-up 2 of D135 (PNO core 1 words only; the FX cores and a captured recall with `kLeslieEnable` were not tried).
3. The amp type edit rewrites the `0xB0476` stage set, which fits the Amp module (drive plus cabinet) of [fx-type-map](fx-type-map.md); this lowers, but does not exclude, the reading of the sets as Leslie family.
4. The stage can be studied without the gate: `leslie_probe` runs it with any signal and speed pair; the drum rate showed no amplitude line in left-lane sines (ASSUMED reading, no phase or Doppler measure yet), which should be checked with a phase / Doppler measure before the engine's rotor and horn roles (horn word to the treble band, rotor to the bass) are trusted.
5. No default, golden or `render_hash` changes; `--leslie firmware` stays opt-in (D22, D63).

## 6. Follow-ups

1. Drive the System routing ids (`0x4e`, `0x59`) and `0x96` / `0xdf` through the system store (`arm_edit_capture` store of domain `0x8` / `0x9`), then re-run the row 6 watch; the routing of the organ to a Leslie bus is the remaining candidate gate.
2. Run the part command consumer (`[part + 0xC8]` list, node handler `0xC010E774`) with the read-back stub answered from the oracle (D139 follow-up 1); the live ARM link of D138 / D144 is the way to do it.
3. Phase / Doppler measure of the stage (instantaneous frequency of a sine through `leslie_probe`) to give the drum rate and its depth, and a second hardware clip with a held fast speed for the physical rate ratio.
4. Set `ROTARY_CROSSOVER_HZ` and the bass / horn gains from a measured crossover only after follow-up 3.

## Reproduce

```bash
# amp type 7 / 0 / 7 on the organ FX unit, row 6 watch (about 70 s)
C=target/fw_unpacked
uv run --with unicorn python tools/arm_table_watch.py $C/bank0_CODE.bin $C/bank0_SRAM.bin \
    --eeprom target/oled/eeprom.bin --range 0xC0545128:0xC0545148 \
    --edit 0x1a7=7 --edit 0x1a7=0 --edit 0x1a7=7 "factory_programs/Program/Bank A/B3 Chords n Solo.ns4p"
# longer recall
uv run --with unicorn python tools/arm_table_watch.py $C/bank0_CODE.bin $C/bank0_SRAM.bin \
    --eeprom target/oled/eeprom.bin --range 0xC0545128:0xC0545148 --ticks 6000 \
    "factory_programs/Program/Bank A/B3 Chords n Solo.ns4p"
# the stage in the oracle port (table of section 2; about 6 min; output committed as data/leslie-probe.txt)
cargo run --release -p nord-tools --bin leslie_probe
# constructor callers
python tools/arm_xref.py callers $C/bank0_CODE.bin 0xC0227D60
# clip envelope lines: cut organ_b3 (238 s, 25 s) with ffmpeg and take the Hilbert envelope per band
```
