# Silent programs in the chain census: 17 of 18 are harness artefacts or legitimate silence, native is not wrong

- **Checked:** 2026-11-18
- **Sources (local, never committed):** `Nord Stage 4 Factory Restore RevK v1.66.ns4b`, OS v1.66 `target/fw_unpacked/bank0_CODE.bin` and `bank0_SRAM.bin`, factory restore EEPROM, `target/PNO/core{1,2}_sram.bin`, `target/oracle/sy.bin`, `factory_programs/Program/`. Tools: `tools/program_expect.py` (new), `tools/program_rank_chain.py`, `tools/arm_dsp_live.py`, `tools/arm_dsp_route.py` (L2 forwarding, new), `tools/arm_zone_map.py`. Data (numbers only): [silent-programs.csv](data/silent-programs.csv). Earlier notes: [program-ranking](program-ranking.md), [zone-key-ranges](zone-key-ranges.md), [engine-routing](engine-routing.md), [arp-fw-vs-native](arp-fw-vs-native.md), [synth-stage-models-12](synth-stage-models-12.md).
- **Question (OQ40):** why does the live ARM + SHARC chain voice nothing at key 60 for 13 synth and 5 organ factory programs, and is the native engine wrong too or only the chain / harness?

Tags follow `.agents/rules/provenance-in-code.md`: **VERIFIED** = reproduced by the commands in "Reproduce", **EMPIRICAL** = measured in this harness, **ASSUMED** = unproven.

## 1. Short answer

| Class | Programs | Cause | Native | Tag |
|---|---|---|---|---|
| Legitimate silence | 5 organ: Ballad Layer Whl, J3 Grand Layers, Wormhole Organ, Nox Split, 24 Frames J3 | the firmware itself keeps the organ layer silent at key 60: zone ends below the key (3) or stored level 0 with no morph (2) | plays only the other layers: correct | VERIFIED (section 3) |
| Harness: wavetable never reaches PNO core 2 | 10 synth: Dance Lead, Broken Chip, 70s CP Strings, Super Org Punch, 90s Organ, Dreamy Syn Tines, Scream Lead AT, Shiny Synth Keys, Calling Whl, Gravity Whl | SUPER 2..4 and WAVETABLE stages read the per-program wavetable at `DM(0x08030000)`; the ARM writes it to the L2 byte window, the Router dropped it | plays: correct | VERIFIED (section 4) |
| Harness: hold shorter than the arp start | 4 synth: Gravity Pattern, Moving Chords, Calling Whl, Gravity Whl (the last two also in the row above) | the first arp step reaches the DSP about 2,900 ARM ticks after the key; the hold was 1,500 | plays: correct | VERIFIED (section 5) |
| Open | 1 synth: Synth Engine | arp mode 1 (zigzag), custom 14 step pattern: the chain stays at 0 even with a 60,000 tick hold | plays; the firmware voice is unexplained | EMPIRICAL, open (section 6) |

Counts: 5 + 10 + 2 (Gravity Pattern, Moving Chords: arp only) + 1 = 18. After the harness fixes the census flags 1 synth program (Synth Engine) and 0 organ programs as chain defects; the 5 organ programs read `organ_expected_silent`.

**Is native also wrong? No.** Native renders every one of the 18 at key 60 (and at the other keys tried), and for the 5 organ programs it also plays only the layers the firmware lets sound. No native change followed from this diagnosis. **No audible native change.** No default flipped.

## 2. What the census measured, and what it could not know

The census (`tools/program_rank_chain.py`) flags an engine silent when its tap stays 0 for one key 60 (velocity 100), 1,500 ticks held. A silent tap is a defect only if the firmware would sound that engine at that key. Nothing in the census computed that. `tools/program_expect.py` now does, from the decoded program (`ns4 inspect --json`), in four steps: scene I mask with the loader rule `default_layer_a` (an enabled section with no layer plays layer A, `engine/routing.rs`), layer key range through `arm_zone_map.py` (VERIFIED on 384 of 384 programs, [zone-key-ranges](zone-key-ranges.md)), stored layer level 0 without morph, arp on or off. `program_rank.py --expect` turns a silent tap into `organ_expected_silent` / `synth_expected_silent` when the expectation says silent, and `program_rank_chain.py --expect` holds arp programs 15,000 ticks.

## 3. Organ: five legitimate silences

`program_expect.py` row per program at key 60 (`docs/research/data/silent-programs.csv`, columns `expect_organ`, `after_pno1_peak`):

| Program | Organ layer in scene I | Reason | PNO core 1 lane peak |
|---|---|---|---|
| Wormhole Organ | A, zone lower half only | key 60 above the zone | 0.0 |
| Nox Split | A, zone below 60 | zone | 0.0 |
| 24 Frames J3 | A, zone below 60 | zone | 0.0 |
| Ballad Layer Whl | A, stored level 0 | level | 0.0 |
| J3 Grand Layers | A, stored level 0 | level | 0.0 |

Control (EMPIRICAL): the same chain voices Nox Split and Wormhole Organ at key 40 (`--key 40`, PNO core 1 lane peak above 0). The other 33 organ programs voice at key 60. The native default render of each of the five is finite and audible at key 60 (`def_rms_db` in `program-ranking.csv`, copied to `silent-programs.csv`) and comes from the other layers; `apply_program` already honours zone and level (`engine/zone_assign.rs`). Not a native bug.

## 4. Synth: the wavetable never reached PNO core 2

Mechanism (EMPIRICAL, `tools/arm_dsp_route.py` run logs `log_*/PNO2.script`, L2 scan of the ARM store stream):

- Stage `0x124985` (SUPER 2..4) and `0x1247B6` (WAVETABLE) read wavetable words at `DM(0x08030000)`. Both ran in the PNO core 2 trace of Dance Lead and Broken Chip and never wrote the voice record `0xC1F89`.
- The ARM writes the per-program wavetable to the L2 byte window `0x200C0000..` (PNO once, ORG twice, 6 segments). A SUPER layer uploads 8,812 words per segment; the boot table is 985 words.
- `Message.core` files those L2 stores under core 1 with a negative `dm_word`, so the Router skipped them: PNO core 2 ran the program with the boot wavetable only.

Fix (harness only): `Router.dispatch` forwards a path 2, core 1, negative `dm_word` store to `forward_l2`, which pokes it into the PNO core 2 chip at `SYS_BASE + 4 * dm_word` (`Link.poke`, logged for replays, not counted as an ARM message). Test: `tools/tests/test_arm_dsp_route.py`, `test_arm_dsp_link.py`.

Result (EMPIRICAL, PNO core 2 voice window peak, key 60, same program, before / after):

| Program | Before | After |
|---|---|---|
| Dance Lead | 0.0 | 2.775 |
| Broken Chip | 0.0 | 1.494 |
| 70s CP Strings | 0.0 | 2.376 |
| Super Org Punch | 0.0 | 2.528 |
| 90s Organ | 0.0 | 1.600 |
| Dreamy Syn Tines | 0.0 | 1.499 |
| Scream Lead AT | 0.0 | 2.555 |
| Shiny Synth Keys | 0.0 | 2.779 |
| Calling Whl | 0.0 | 1.264 (15,000 tick hold) |
| Gravity Whl | 0.0 | 2.780 (15,000 tick hold) |

70s CP Strings has no stored layer in scene I; the loader rule `default_layer_a` plays synth layer A (a Wavetable), which is why a first reading called it empty. L2 forwarding changes the chain of every program that uploads a wavetable. The 384-program census must be rerun before the earlier rows of `program-ranking.csv` are read again (follow-up 1).

## 5. Arp: the first step arrives late

Gravity Pattern (arp on) has a first arp key word about 2,900 ticks after the key (`log_gp*/PNO2.script`, EMPIRICAL), so a 1,500 tick hold ends before it. With `ARP_KEY_TICKS = 15,000` (0.625 s, 1.25 beats at 120 bpm): Gravity Pattern 0.167, Moving Chords 0.246, Calling Whl 1.264, Gravity Whl 2.780. Same method as [arp-fw-vs-native](arp-fw-vs-native.md): the arp is the firmware's own player, no input forced.

## 6. Open: Synth Engine

Layer A of Synth Engine: arp mode 1 (zigzag), 14 step custom pattern, gate word `0x20000000` (2 bit steps, `0` = eighth per `cbin/synth_pattern.rs`), rate raw 68, 154 bpm, level 1.0, filter Lp24 659 Hz. The chain stays at 0.0 with holds of 1,500, 6,000, 15,000 and 60,000 ticks (`se4`, `se5` runs: PNO core 2 receives 13,511 / 13,535 messages, window peak 0.0, no error). Gravity Pattern, same harness, mode 0, voices. Native sounds (-32.8 dB RMS default render). Whether the firmware sounds a zigzag custom pattern at a single key (an ARM-side gate we do not model) or the harness misses a record is not decided. This is the only remaining chain defect of the 18 and stays open as OQ40 (narrowed).

## Reproduce

```bash
python tools/program_expect.py --programs factory_programs/Program --ns4 target/release/ns4 \
    --key 60 --out target/rank/expect.jsonl
uv run --with unicorn --with capstone python tools/program_rank_chain.py \
    --programs factory_programs/Program --out target/rank/chain --workers 8 \
    --fw-dir target/fw_unpacked --eeprom target/oled/eeprom.bin --backup "$B" \
    --pno1 target/PNO/core1_sram.bin --pno2 target/PNO/core2_sram.bin \
    --sy target/oracle/sy.bin --live target/release/sharc-live \
    --native target/rank/native.jsonl --expect target/rank/expect.jsonl
python tools/program_rank.py --native target/rank/native.jsonl --chain target/rank/chain/chain.jsonl \
    --expect target/rank/expect.jsonl --csv docs/research/data/program-ranking.csv
```

`docs/research/data/silent-programs.csv` holds the 18 rows: class, cause, native RMS, before and after peaks, hold length, expectation, flags.

## Implications

- OQ40 is answered for 17 of 18 programs; the native engine is not wrong for any of them. The ranking in [program-ranking](program-ranking.md) stands; only its chain columns for wavetable programs were wrong.
- The chain harness had two blind spots: a stale L2 route (wavetable) and a fixed 1,500 tick hold (arp). Both are fixed; a silent tap is now compared to the firmware expectation before it is called a defect.
- A silent chain tap says nothing about native audio: the piano programs still have no tap (155 programs).

## Follow-ups

1. Rerun the 384-program census with L2 forwarding, `--expect` and the arp hold, then regenerate `program-ranking.csv` (about 1 h at 8 workers); programs that upload a wavetable may change their chain columns.
2. Synth Engine: trace why the zigzag custom pattern gives no voice record (ARM pattern player `0xC023B3EC`, listener `0xC0123A18`, [arp-fw-vs-native](arp-fw-vs-native.md)).
3. A piano tap (PNO core 1 with the SMP stream).
4. Sample programs (Wormhole Organ, Nox Split, 24 Frames J3, J3 Grand Layers also carry SAMPLES layers): their SY path is still `sample_layer_lost` / `sample_voiced_no_audio` (D150, D154), not touched here.
