# Piano chain tap: what the live ARM + SHARC link can and cannot give for the 155 piano programs

- **Checked:** 2026-11-19
- **Sources (local, never committed):** OS v1.66 `target/fw_unpacked/bank0_CODE.bin`, `bank0_SRAM.bin`, factory restore EEPROM, `target/PNO/core1_sram.bin`, `target/DSP/dsp_image_4_SMP.bin`, `factory_programs/Program/Bank A/Astoria Grand.ns4p`. Tools: `tools/arm_dsp_live.py` (per phase message counts, new), `tools/program_rank_chain.py` (`key_arm_messages`, new). Earlier notes: [piano-dsp-path](piano-dsp-path.md), [piano-sound-load](piano-sound-load.md), [piano-bus-convention](piano-bus-convention.md), [pno-dsp-map](pno-dsp-map.md) s1 and s7, [sy-smp-audio](sy-smp-audio.md), [smp-voice](smp-voice.md), [arm-dsp-live-link](arm-dsp-live-link.md), [program-ranking](program-ranking.md).
- **Question (OQ41):** which chip and core carries the piano voice in the live link, can a tap be built with the current harness, and if not, what exactly blocks it?

Tags follow `.agents/rules/provenance-in-code.md`: **VERIFIED** = reproduced by the commands in "Reproduce", **EMPIRICAL** = measured in this harness or read from earlier notes, **ASSUMED** = reading.

## 1. Short answer

| Question | Result | Tag |
|---|---|---|
| Which chip plays piano? | The **SMP** image (ARM comm index 4, path 1: DM word addresses, FIFO base `0x92196`, 114 voice records of 54 words from `0x92848`). PNO core 1 is block level stereo post processing; it has no sample voice and its lane `2:0` is the organ lane. The 155 census runs put a `pno` chip on core 1 only as a placeholder, so their `pno1_peak` says nothing about piano. | EMPIRICAL (code reading in [pno-dsp-map](pno-dsp-map.md), [smp-voice](smp-voice.md); run below) |
| Does a piano key reach the SMP chip in the census harness? | **No.** Astoria Grand, key 60 velocity 100, 1,500 ticks held: SMP messages (comm 4, path 1) by phase: boot 24, load 1,516, idle 0, key **0**, release **0** (1,540 in all, the figure of [arm-dsp-live-link](arm-dsp-live-link.md) s5). The key sends nothing to any sample chip. | EMPIRICAL (run 2026-11-19; also 12 of 12 piano programs of the sample, section 3) |
| Why not? | The census recalls the program but the harness has no sound library: no piano sound object is bound to the core, so the key handler `0xC00BC8AC` never reaches the voice start `0xC00A9274` ([piano-sound-load](piano-sound-load.md) s2 and s3: with a loaded and bound object the same key sends 58 SMP messages in 4 ticks). | EMPIRICAL (earlier run, not repeated here) |
| Can a tap be built now? | **No, four things are missing** (section 2). Nothing was guessed: no piano column was added to the ranking, the flag `piano_no_chain_tap` stays. | EMPIRICAL |

## 2. What blocks a piano tap

| # | Blocker | What exists | What is missing |
|---|---|---|---|
| 1 | Piano sound object | `tools/arm_piano_load.py`: `LibraryStub`, `load_sound`, `select_sound`, `install_dsp_ready` run on the settled state of `arm_keydown` | the live driver does not call them; the map from the program's stored piano sound id to a backup `.npno` member is unknown (the sample path binds an `.nsmp4` under an ASSUMED id, `arm_dsp_capture.bind_samples`); `select_sound` binds layer 0 of core 0, not the program's piano layers 2 and 3 |
| 2 | An SMP chip in `sharc-live` | the SMP image runs under `sharc-ref` scenarios (FIFO, audio interrupt, voice loop, [sy-smp-audio](sy-smp-audio.md)); the SY model of `runner/live_sy.rs` is the template | `preset_named("smp")` is `None` (asserted by a unit test): FIFO base `0x92196`, no Stage A state words, a piano record layout instead of the 56 voice SY records |
| 3 | Page data | SMP plays real CNSP pages bit exactly when a harness pre-fills the ring ([smp-voice](smp-voice.md)) | the external port DMA that refills the source ring from SDRAM is not modelled; the SY model fills with a header word that makes control flow run, which gives no level. A level needs the pages of the sound the ARM asks for |
| 4 | The tap point | the SMP post stage ring ([piano-bus-convention](piano-bus-convention.md) s1) | the ring is consumed by the FX chip (SPORT3B, [piano-receiver-scale](piano-receiver-scale.md)), which is not a live chip; the FX hand-over is outside every tap |

The PNO lane hand-over `0x1229B3` named in the brief is the organ output stage ([organ-model-paths](organ-model-paths.md) s4: Vox record `I1 = 0xE2D42`); no document ties it to a piano voice, so it is not a piano tap.

## 3. What was built

Only the measurement that makes the gap reproducible: `arm_dsp_live.py` reports the ARM messages of every phase (`phases.<phase>.arm_messages`, all comms), `program_rank_chain.py` keeps the key phase counts (`key_arm_messages`) in `chain.jsonl` (data file, not the CSV: its schema stays stable for the ranking consumers). A sample of 12 of the 155 piano programs (every 13th, `--entries`) was run with it: **no key phase message to SMP in any of the 12** (key phase totals 18 to 20 messages, all to PNO cores; Vibra Band, a piano+sample program, sends 41 to SY from its SAMPLE layer, none to SMP). EMPIRICAL; the other 143 were not run (about 15 minutes per 12 under load).

## Reproduce

```bash
python tools/arm_dsp_live.py $C/bank0_CODE.bin $C/bank0_SRAM.bin --eeprom target/oled/eeprom.bin \
    --program "factory_programs/Program/Bank A/Astoria Grand.ns4p" --live target/release/sharc-live \
    --poll-advance 64 --chip "PNO:1=pno=target/PNO/core1_sram.bin" --out astoria.json
python -c "import json; r = json.load(open('astoria.json')); print({p: r['phases'][p]['arm_messages'] for p in ('load', 'key', 'off')})"
```

## Implications

- The ranking cannot compare native and chain for piano until blockers 1 to 3 are done; the flag stays honest (`piano_no_chain_tap`).
- Blocker 1 alone would give a **voice-start tap** (SMP key messages, record gain words: velocity law, voice count) without audio; blockers 2 and 3 give audio. That is a separate task (bean `ns4-pnoc`).

## Update 2026-11-21 (D185)

Blockers 1 and 2 are done: [piano-chain-tap-smp](piano-chain-tap-smp.md). A bound piano sound makes the key send 70 to 117 messages to a live SMP chip; the tap is silent until the page refill (blocker 3).

## Follow-ups

1. Bind a piano sound in the live driver (`arm_piano_load` stub), program to `.npno` map from the stored sound id.
2. `smp` preset and model in `sharc-live` (template `live_sy.rs`).
3. CNSP page refill for the SMP ring from the bound `.npno`.
4. FX chip live (OQ36) to tap the SPORT3B hand-over.
