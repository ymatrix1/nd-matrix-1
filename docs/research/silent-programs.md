# Silent programs in the chain census: 17 of 18 are harness artefacts or legitimate silence; the 18th (Synth Engine) is a right-panned arp step that the left-lane census window missed (D175), native matches the firmware player

- **Checked:** 2026-11-18, reviewed and extended 2026-11-19 (D170)
- **Sources (local, never committed):** `Nord Stage 4 Factory Restore RevK v1.66.ns4b`, OS v1.66 `target/fw_unpacked/bank0_CODE.bin` and `bank0_SRAM.bin`, factory restore EEPROM, `target/PNO/core{1,2}_sram.bin`, `target/oracle/sy.bin`, `factory_programs/Program/`. Tools: `tools/program_expect.py` (new), `tools/program_rank_chain.py`, `tools/arm_dsp_live.py`, `tools/arm_dsp_route.py` (L2 forwarding, new), `tools/arm_zone_map.py`. Data (numbers only): [silent-programs.csv](data/silent-programs.csv). Earlier notes: [program-ranking](program-ranking.md), [zone-key-ranges](zone-key-ranges.md), [engine-routing](engine-routing.md), [arp-fw-vs-native](arp-fw-vs-native.md), [synth-stage-models-12](synth-stage-models-12.md).
- **Question (OQ40):** why does the live ARM + SHARC chain voice nothing at key 60 for 13 synth and 5 organ factory programs, and is the native engine wrong too or only the chain / harness?

Tags follow `.agents/rules/provenance-in-code.md`: **VERIFIED** = reproduced by a command committed in "Reproduce" (only the firmware expectation of `tools/program_expect.py` qualifies; the live ARM runs take 2 to 100 minutes and their logs are not committed, so every chain measurement is **EMPIRICAL**), **ASSUMED** = unproven.

## 1. Short answer

| Class | Programs | Cause | Native | Tag |
|---|---|---|---|---|
| Legitimate silence | 5 organ: Ballad Layer Whl, J3 Grand Layers, Wormhole Organ, Nox Split, 24 Frames J3 | the firmware itself keeps the organ layer silent at key 60: zone ends below the key (3) or stored level 0 with no morph (2) | plays only the other layers: correct | expectation VERIFIED (`program_expect.py`), chain silence EMPIRICAL (section 3) |
| Harness: wavetable never reaches PNO core 2 | 10 synth: Dance Lead, Broken Chip, 70s CP Strings, Super Org Punch, 90s Organ, Dreamy Syn Tines, Scream Lead AT, Shiny Synth Keys, Calling Whl, Gravity Whl | SUPER 2..4 and WAVETABLE stages read the per-program wavetable at `DM(0x08030000)`; the ARM writes it to the L2 byte window, the Router dropped it | plays: correct | EMPIRICAL (section 4) |
| Harness: hold shorter than the arp start | 4 synth: Gravity Pattern, Moving Chords, Calling Whl, Gravity Whl (the last two also in the row above) | the first arp step reaches the DSP about 2,900 ARM ticks after the key; the hold was 1,500 | plays: correct | EMPIRICAL (section 5) |
| DSP key word | 1 synth: Synth Engine | arp mode 1 (zigzag), custom 14 step pattern: the first step is pan R, key word `0x0100E200` = right lane; the census window read the left lane only | harness window, fixed (`0xC1F89:8:1`); native plays the step, its hard pan gain was 3 dB high (D175) | VERIFIED (section 6, [arp-pan-lanes](arp-pan-lanes.md)) |

Counts: 5 + 10 + 2 (Gravity Pattern, Moving Chords: arp only) + 1 = 18. After the harness fixes the census flags 0 synth and 0 organ programs as chain defects (the two-lane window rerun of 2026-11-20 clears Synth Engine); the 5 organ programs read `organ_expected_silent`.

**Is native also wrong? Not shown for any of the 18.** Native renders every one of the 18 at key 60, and for the 5 organ programs it plays only the layers the firmware lets sound. For Synth Engine native plays what the firmware player hands out (section 6); whether the hardware sounds that step is not decided, so "native is right" is not claimed there. No native change followed. **No audible native change.** No default flipped.

## 2. What the census measured, and what it could not know

The census (`tools/program_rank_chain.py`) flags an engine silent when its tap stays 0 for one key 60 (velocity 100), 1,500 ticks held. A silent tap is a defect only if the firmware would sound that engine at that key. Nothing in the census computed that. `tools/program_expect.py` now does, from the decoded program (`ns4 inspect --json`), in four steps: scene I mask with the loader rule `default_layer_a` (an enabled section with no layer plays layer A, `engine/routing.rs`), layer key range through `arm_zone_map.py` (VERIFIED on 384 of 384 programs, [zone-key-ranges](zone-key-ranges.md)), stored layer level 0 without morph, arp on or off. `program_rank.py --expect` turns a silent tap into `organ_expected_silent` / `synth_expected_silent` when the expectation says silent, and `program_rank_chain.py --expect` holds arp programs 15,000 ticks.

## 3. Organ: five legitimate silences

`program_expect.py` row per program at key 60 (`docs/research/data/silent-programs.csv`, columns `expect_organ`, `after_pno1_peak`):

| Program | Organ layer in scene I | Reason | PNO core 1 lane peak |
|---|---|---|---|
| Wormhole Organ | A, zone lower half only | key 60 above the zone | 0.0 |
| Nox Split | A, zone below 60 | zone | 0.0 |
| 24 Frames J3 | A, zone gain 0 at key 60, stored level 0 | zone and level (either one silences it) | 0.0 |
| Ballad Layer Whl | A, stored level 0 | level | 0.0 |
| J3 Grand Layers | A, stored level 0 | level | 0.0 |

Controls (EMPIRICAL, `--key 40`, reproduced 2026-11-19): the chain voices Wormhole Organ at key 40 (PNO core 1 lane peak 0.0912) and Nox Split (earlier run). The other three have **no positive control**: Ballad Layer Whl and J3 Grand Layers store level 0 for the organ layer, so no key can sound it, and 24 Frames J3 stores level 0 as well (its zone gain is 1.0 at key 40, level 0.0; `program_expect.py --key 40`), the chain reads 0.0 at key 40 for all three, which agrees with the expectation but proves nothing about the chain. The first version of this note named the zone as the only cause of 24 Frames J3; the level 0 is the cause that does not depend on the key. The other 33 organ programs voice at key 60. The native default render of each of the five is finite and audible at key 60 (`def_rms_db` in `program-ranking.csv`, copied to `silent-programs.csv`) and comes from the other layers; `apply_program` already honours zone and level (`engine/zone_assign.rs`). Not a native bug.

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

## 6. Synth Engine: the step is right-panned, the census window read the left lane (answered, D175)

**Answer (2026-11-20, [arp-pan-lanes](arp-pan-lanes.md), VERIFIED):** the two low bits of the key word are the output lane select of the voice record (`0x01` right = odd words, `0x02` left = even words, `0x03` both), set by the pattern pan of the step (`CRLCRC..`; the ARM plays R, L, C, R); the window `0xC1F89:4:2` reads the even words only. The hardware sounds the step; the window `0xC1F89:8:1` reads peak 0.1944 live. The text below is the first reading (EMPIRICAL, bank-bit wording superseded).

Layer A: arp mode 1 (zigzag), 14 step custom pattern (all 14 steps `E`: gate word `0x20000000` sets only step 14, which lies beyond the length), rate raw 68, file BPM 154, no MST CLK, level 1.0, filter Lp24 659 Hz. Findings (EMPIRICAL, runs of 2026-11-19; `tools/arm_key_words.py` logs the key words):

| Step | Result |
|---|---|
| Does the ARM play the note? | Yes. The firmware pattern player (`tools/arm_arp_events.py`, 3 s) hands 30 events to the listener; the voice list holds note 60 from +3,794 ticks to +6,898 ticks, again every 4,672 ticks (194.7 ms = 30 / 154 s), the same behaviour as Gravity Pattern (mode 0, voices). |
| What reaches PNO core 2 in the live link (8,000 tick hold) | two key words: `0x0100E200` at tick +3,796 (on) and `0x0000E200` at +6,900 (off). Gravity Pattern: `0x03F4E200` (on, note 48), `0x00F4E200` (off). A plain key: on `0x0B..`, off `0x08..` (`arm_key_words.py` fields). Synth Engine's on word has bank bit `0x01` only; the voices that sound carry `0x03`. |
| Is that the reason for silence? | Yes at the DSP level. The same run with the router setting bit `0x02` of the on word (`0x0300E200`) gives voice window peak **0.1944** (silent run 0.0, Gravity Pattern 0.1669). A chord (60, 64, 67, 12,000 ticks) alternates banks `0x01` then `0x02` and stays 0.0 in the window (the window sees voice 0 only). |
| Does native play the same? | Note, step and release yes. `arp_trace` (native, `Synth Engine.ns4p`, key 60): note 60 every 194.8 ms, 97.4 ms on. Firmware: note 60 every 194.7 ms, on for 3,104 ticks = 129.3 ms (2/3 of a step, the known gate difference of D162, opt-in `--arp-fw-gate`); first note at +158 ms against +194.8 ms native (known capture phase, [arp-fw-vs-native](arp-fw-vs-native.md)). The custom zigzag pattern with one key is a repeated note 60 in both. No native change. |

Superseded by the answer above (it was: why the ARM leaves bank bit `0x02` clear for this step (the key word builder is near `0xC01EC690`; the per-layer flag word `[table + 0xD0 + 4 i]` tested with `tst lr, #2` is the candidate), and whether the hardware sounds the step. Alternation of the bank bits in the chord run suggests round robin voice banks, so a single note may land on a bank the 0xC1F89 window does not show; the window taps voice 0 of one bank only (D144). The builder trace and the lane readout answered it.)

## Reproduce

```bash
M=.   # main checkout with target/, .reference/, factory_programs/
B="$M/.reference/Nord Stage 4 Factory Restore RevK v1.66.ns4b"
OUT=$(mktemp -d)   # a fresh directory: the census resumes from OUT/chain.jsonl
python tools/program_expect.py --programs $M/factory_programs/Program --ns4 target/release/ns4     --key 60 --out $OUT/expect.jsonl
target/release/program_rank "$B" > $OUT/native.jsonl
uv run --with unicorn --with capstone python tools/program_rank_chain.py     --programs $M/factory_programs/Program --out $OUT/chain --workers 8     --fw-dir $M/target/fw_unpacked --eeprom $M/target/oled/eeprom.bin --backup "$B"     --pno1 $M/target/PNO/core1_sram.bin --pno2 $M/target/PNO/core2_sram.bin     --sy $M/target/oracle/sy.bin --live target/release/sharc-live     --native $OUT/native.jsonl --expect $OUT/expect.jsonl --audio-for $OUT/audio.txt
python tools/program_rank.py --native $OUT/native.jsonl --chain $OUT/chain/chain.jsonl     --expect $OUT/expect.jsonl --csv docs/research/data/program-ranking.csv
# key 40 control of an organ program, and the Synth Engine key words (PNO core 2)
python tools/arm_dsp_live.py $M/target/fw_unpacked/bank0_CODE.bin $M/target/fw_unpacked/bank0_SRAM.bin     --eeprom $M/target/oled/eeprom.bin --program "$M/factory_programs/Program/Bank B/Wormhole Organ.ns4p"     --live target/release/sharc-live --poll-advance 64 --key 40 --chip "PNO:1=pno=$M/target/PNO/core1_sram.bin"
python tools/arm_key_words.py 1 key_words.json $M/target/fw_unpacked/bank0_CODE.bin     $M/target/fw_unpacked/bank0_SRAM.bin --eeprom $M/target/oled/eeprom.bin     --program "$M/factory_programs/Program/Bank F/Synth Engine.ns4p" --live target/release/sharc-live     --poll-advance 64 --key-ticks 8000 --chip "PNO:2=org=$M/target/PNO/core2_sram.bin=0xC1F89:4:2"
target/release/arp_trace "Synth Engine.ns4p" --keys 60 --seconds 2 --tempo 120   # run in the Bank F folder
```

`$audio.txt` lists the 31 SAMPLE programs of the SY return path run (the entries with `sy_audio_peak` in the CSV). A failed census entry is retried on a rerun into the same `--out`.

`docs/research/data/silent-programs.csv` holds the 18 rows: class, cause, native RMS, before and after peaks, hold length, expectation, flags.

## 7. Census rerun (2026-11-19, D170)

All 384 programs through the chain with the L2 forwarding, `--expect` and the arp hold, native rendered on the current defaults (D163 to D166); the scoring columns of `program-ranking.csv` are byte-identical to `main` (0 of 384 differ), only the chain columns changed. `program_rank_chain.py` now retries failed entries (41 census failures of an interrupted first pass, 4 missing return-path runs; the old resume skipped them). EMPIRICAL.

| Change | Programs |
|---|---|
| Flag `synth_chain_silent` gone, now voiced | 12: Dance Lead, Gravity Pattern, 70s CP Strings, Broken Chip, Calling Whl, Super Org Punch, 90s Organ, Dreamy Syn Tines, Gravity Whl, Moving Chords, Scream Lead AT, Shiny Synth Keys |
| Still `synth_chain_silent` | 0 (Synth Engine voices with the two-lane window, peak 0.1944; census rerun 2026-11-20, D175) |
| `organ_chain_silent` becomes `organ_expected_silent` | 5 |
| Window peak 0.0 to above 0 without a flag | 1: Bell Strings (1.349), not flagged in the first census |
| Window peak level change, both runs voiced | 13 programs: above 1 dB 7 (Night Gaunts Whl +19.7 dB, Boxxx +14.1, Mirror Image Pad +3.5, Unclocked Pad +3.6, Dual Gates +3.0, FM Cloud Whl +2.4, Horizon Whl +1.6), below 1 dB 6 |
| Other flags (`sample_*`, `piano_no_chain_tap`) | unchanged (`sample_layer_lost` 24, `sample_no_voice` 3, `sample_voiced_no_audio` 19, `piano_no_chain_tap` 155) |

The level changes come from the wavetable (L2 forwarding) and the longer hold of arp programs. 144 peak cells (of the three peak columns) are empty now where the first census had a `0.0`: the chip of that column is not run for the engines of the program (`chips_for`), so `0.0` always means a run that read silence. Programs in the piano sample (12 of 155, every 13th): no SMP key message, [piano-chain-tap](piano-chain-tap.md).

## 8. Census rerun with the two-lane window (2026-11-20, D175)

All 384 programs, window `0xC1F89:8:1`, native on `main` of 2026-11-20 (`program_rank`, `program_expect`, `program_rank_chain.py` as in Reproduce; 113 entries failed the first pass because a stale `sharc-live` lacked `--sy` and were retried). EMPIRICAL.

| Change against the census of 2026-11-19 | Result |
|---|---|
| Flag `synth_chain_silent` | Synth Engine gone (peak 0.1944); no other flag changes (0 synth, 0 organ defects) |
| Chain peaks (`pno2_peak`) | 52 of 384 change, all because the window reads the right lane too (the peak is the larger of the two lanes): Synth Engine 0.0 to 0.194, the others up where the right lane is louder (largest Gray Pad Whl 0.516 to 0.750, Danceflr Peak MW 0.310 to 0.455, Lush Pluck Whl 0.313 to 0.403) |
| Native score (`score`) | 30 programs change, **280 of 384 ranks shift**, because the 30 scores move (largest: J3 Super Sawz 12.92 to 3.62, rank 14 to 144; Sidechain String 6.16 to 1.05, rank 72 to 300; Boxxx 7.29 to 4.32, rank 47 to 125). Maximum rank shift 228; the other 270 shifts are one to a few places, caused by those 30 |

The score changes come from native changes after the CSV of 2026-11-19 (arp 2/3 gate default D168, D175 pan, pump / filter work on `main`), not from the window: the window only changes chain columns.

## Implications

- OQ40 is answered for all 18 programs (the last one in [arp-pan-lanes](arp-pan-lanes.md), D175); the native engine misses no note for any of them (its hard pan gain was 3 dB high, D175). The ranking in [program-ranking](program-ranking.md) stands; only its chain columns for wavetable programs were wrong.
- The chain harness had two blind spots: a stale L2 route (wavetable) and a fixed 1,500 tick hold (arp). Both are fixed; a silent tap is now compared to the firmware expectation before it is called a defect.
- A silent chain tap says nothing about native audio: the piano programs still have no tap (155 programs).

## Follow-ups

1. ~~Rerun the 384-program census with L2 forwarding~~ done 2026-11-19 (section 7).
2. ~~Synth Engine: trace the key word builder~~ done 2026-11-20 (section 6, [arp-pan-lanes](arp-pan-lanes.md), D175). Census rerun with the two-lane window `0xC1F89:8:1` done 2026-11-20 (section 8).
3. A piano tap: not feasible with the harness, blockers in [piano-chain-tap](piano-chain-tap.md) (OQ41).
4. Sample programs (Wormhole Organ, Nox Split, 24 Frames J3, J3 Grand Layers also carry SAMPLES layers): their SY path is still `sample_layer_lost` / `sample_voiced_no_audio` (D150, D154), not touched here.
