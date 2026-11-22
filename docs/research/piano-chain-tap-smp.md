# Piano chain tap, steps 1 and 2: a bound piano sound reaches a live SMP chip (D185)

- **Checked:** 2026-11-21
- **Sources (local, never committed):** OS v1.66 `target/fw_unpacked/bank0_CODE.bin`, `bank0_SRAM.bin`, factory restore EEPROM, `target/DSP/dsp_image_4_SMP.bin` (flat form `target/oracle/smp.bin` from `python tools/ns4_dsp_raw.py ... --sram`), `target/PNO/core1_sram.bin`, factory backup `Nord Stage 4 Factory Restore RevK v1.66.ns4b` (`.npno` members, read only). Code: `tools/arm_piano_bind.py` (new), `tools/arm_dsp_live.py --piano-bind`, `tools/arm_dsp_route.py` (`SMP:0=smp=IMAGE`), `crates/nord-sharc/src/runner/live_smp.rs` (new). Earlier notes: [piano-chain-tap](piano-chain-tap.md) (OQ41, D170), [piano-sound-load](piano-sound-load.md), [smp-voice](smp-voice.md), [sy-smp-audio](sy-smp-audio.md), [piano-bus-convention](piano-bus-convention.md), [dsp-flash-dma-model](dsp-flash-dma-model.md).
- **Question (OQ41):** can a piano key press reach the SMP chip in the live ARM + SHARC link, can the chain tap show piano audio, and what is the next blocker?

Tags follow `.agents/rules/provenance-in-code.md`: **VERIFIED** = reproduced by the commands in "Reproduce", **EMPIRICAL** = measured in this harness, **ASSUMED** = reading.

## 1. Short answer

| Question | Result | Tag |
|---|---|---|
| Blocker 1: piano sound object | **Done.** `arm_piano_bind` maps the program's piano layer `name_hash` to the `.npno` member of the backup (330 of 330 enabled factory piano layers resolve), serves it as library entry 1, loads it with the firmware loader and binds it to core 0 layer 0 (`arm_piano_load`). Loader status 0, `[core + 4]` = the pool object | VERIFIED (run, 4 programs) |
| Does the key now reach SMP? | **Yes.** Key phase SMP messages (comm 4, path 1): Astoria Grand 117 (115 delivered to the chip; 495 over 1,500 ticks), Classic Wah Clav 70, Felt Upright 117, Bubbles Mk I 111; the census harness sent 0 ([piano-chain-tap](piano-chain-tap.md) s1). The release sends 22 to 71 more | VERIFIED (`data/piano-chain-tap-smp.csv`) |
| Blocker 2: SMP chip in `sharc-live` | **Done.** `sharc-live --smp` (preset `smp`): entry `0x1C11E0`, `DM(0x5201) = 0x40`, FIFO base `0x92196`, the SY pass model (vector 13 batch, vector 17 frame) with no state-word staging, output read from the post stage ring. 1,446 passes per run, 0 errors, 4 of 4 programs | VERIFIED (unit tests; run) |
| Does the tap show piano audio? | **No.** SMP lane peak 0.0 for Astoria Grand, Felt Upright and Bubbles Mk I; Classic Wah Clav reads 2.6e-4, the size of the post stage gate fade with no bus input ([piano-bus-convention](piano-bus-convention.md) s2b: `ctl only` 0.0001, 0.0004, ...), so it is not sample audio. The voices decode an empty source ring | EMPIRICAL |
| Blocker 3: page refill | **Open.** The ARM sends the stroke descriptor words of a voice (s3) but no page data; the pages come from the external port DMA of the chip, which has no external image in the harness | EMPIRICAL |
| Blocker 4: tap point | The post stage ring `0xB2000` / `0xB2014` (2 stereo frames per interrupt, float `bus * 2^-29`) is read by `live_smp::ring_frames`; which half an interrupt writes is ASSUMED (the half the pointer `DM(0xB2028)` holds on entry), untested on the real image because the ring is silent | ASSUMED |

## 2. What was built

| Piece | Content | Test |
|---|---|---|
| `tools/arm_piano_bind.py` | `nord_name_hash` (mirror of `nord_name_hash`, ARM0 `0x0a5a44`), `resolve_member`, `program_piano_layers` (enable bits and `name_hash` read straight from the payload), `find_cores`, `core0_of`, `bind_piano` | `tools/tests/test_arm_piano_bind.py` |
| `tools/arm_dsp_live.py --piano-bind BACKUP` | after the idle phase binds the program's enabled piano layer (`bind_program_piano`) | thin driver, covered by the run |
| `tools/arm_dsp_route.py` | chip `SMP:0=smp=IMAGE`, path 1, flag `--smp` | `test_arm_dsp_route.py` |
| `live_smp.rs`, `live_sy.rs`, `live_presets.rs`, `sharc_live.rs` | `SyModel::smp`, `smp_preset`, `--smp`, ring tap | 4 new Rust unit tests (`live_smp` 3, `live_sy` 1) and 2 extended (`live_presets`, `sharc_live` parse) |

Two findings on the way:

1. **The core object is not at `0x90BF5400` in the full machine.** The constants of `arm_piano_load` come from the settled snapshot; in the `arm_dsp_live` machine the managers are `0x90BF5420` / `0x90BF5430` and core 0 sits `0x30` after manager 0 (as before: `0x90BF53D0` -> `0x90BF5400`). `find_cores` scans for the pool pointer at `+0x14` (eight candidates) and `core0_of` keeps the one `0x30` after manager 0. A first run that bound the first candidate wrote into pool memory and sent only 2 key messages: the choice matters (EMPIRICAL).
2. **`ns4_inspect._decode_piano_layer` reads the piano layer fields 2 bits late** (it seeks to bit 28 before `variation`, the Rust decoder reads `variation` first). `name_hash`, `acoustic`, `touch` of that decoder are wrong for real programs; its own test pins the shifted layout (acoustic at +62, Rust: +60). Not changed here (follow-up 5); `arm_piano_bind` reads `name_hash` directly (bit 1956 of Astoria Grand = `0x444EBB81`, the value `ns4 inspect --json` prints).

## 3. What the key writes to the SMP (Astoria Grand, first 400 key ticks, EMPIRICAL)

All values are ARM message words; no file content is quoted.

| Words (DM) | Content | Reading |
|---|---|---|
| `0x93936/34/33/38` and `+0x36 k` (k = 0..4) | `1.0, 0, 0, g` with `g` = `0.9793`, `0.9486`, `0.9793`, ... | per voice gain / pan block, five records of stride `0x36` |
| `0x9252E..0x92530`, `0x92522..0x92524`, `0x925F6`, `0x92617`, `0x92668..0x92689` | voice list entries (`0x92534`-style pointers), counts, ring pointers `0x9401E`, `0x93928`, `0x93FE8`, `0x9395E` | voice allocation |
| `0xC0500..0xC0507`, `0xC0510..0xC0517` | eight signed halfword values each (`-41, 22, -57, 37, -9, -28`, ...) | predictor / LPC coefficient tables (`COEF` of [smp-voice](smp-voice.md) s2) |
| `0x9391D..0x93924`, `0x93953..0x9395A` | `0x6E6F`, then seven words (`0x9E7E, 0x27AE4, 0x27AC9, 0xD385, 0x1A28C, 0x1AA46, 0x9EC5`) | the stroke descriptor: **record fields of the `.npno` stroke table**. `0xD385` = the halfword 54149 of stroke record 513 of the bound file; `0x1A28C` and `0x1AA46` = the word pairs `(1, 41612)` and `(1, 43590)` of the same record; the file stroke audio offset is the `u32` at +0 of the record |
| `0x9251C..0x92521`, `0x92528..0x9252D` | float words `0xCC5D4800`, `0x3EEC1CE8`, `0x3F7FFE8E`, ... (voice record `+11..+16` gains, slews) | gains: `0x9252D = 0.25` (`0x3E800000`) is the Q23 level of velocity 100 of [piano-sound-load](piano-sound-load.md) s10 |
| `0x93916`, `0x9394C` | `0x9404E` | bus pointer of the voice (record `-18`, bus set `0x9404E` of [piano-bus-convention](piano-bus-convention.md) s2) |
| `0x9403E`, `0x94086/87` | `1.0`, `0`, `0.99999` | post stage gate and one-pole state (the words that open the ring, `piano-bus-convention.md` s2b) |

So the ARM does what `smp-voice.md` s2 expects: it fills the voice record, the coefficient tables, the stroke descriptor, the gains and the bus pointer. The words equal the `smp_voice_ref` scenario fields; what is missing is the data behind the descriptor.

## 4. The next blocker: the source ring has no page data

The voice loop reads the stream ring (`-26`, 256 words, [smp-voice](smp-voice.md) s2). On the hardware the external port DMA chain refills it from the external banks ([dsp-flash-dma-model](dsp-flash-dma-model.md): bank 2 / 3 reads, 8 word B transfers, per chain TCB); the harness has no external image and the SMP model does not run the EP chain, so every voice decodes a ring of zeros and the bus stays 0.

What a refill needs (all open):

| # | Need | State |
|---|---|---|
| a | Which stroke record the voice plays | the descriptor words above carry its fields (record 513 for Astoria Grand key 60 velocity 100); the mapping `descriptor -> record` is a lookup of the pair `(1, 41612)` in the table, not decoded in the other direction |
| b | Where in the external space the page of that stroke lives | the descriptor halfwords `0x27AE4`, `0x27AC9` (and `0x9E7E`, `0x9EC5`) are probably page counters / sizes; their relation to the stroke audio offset (`u32` at +0 of the record, [nord-piano-npno-and-codec](nord-piano-npno-and-codec.md)) is not decoded |
| c | The external image | `.npno` payload at the offset the ARM expects (ASSUMED: the file itself, as loaded to external memory by the ARM loader `0xC00B92E4`; unverified) |
| d | The EP chain on the SMP image | the model exists for the SY (`--epchain`, `core::periph::epchain`); not enabled for `smp` |

Smallest next experiment: serve the bound `.npno` as the external image of `epchain` for the SMP chip (`sharc-live --smp --epchain` plus a file base) and watch the TCB external addresses the SMP ISR builds for the voice; if they land inside `[file base, file base + size)` the mapping (b), (c) falls out of the first chain, otherwise the log gives the offset law.

## 5. Not done

- No audio tap, so no comparison with the native piano and no ranking column: the flag `piano_no_chain_tap` stays on all 155 programs (a bound sound exists only in this driver, the census does not call it).
- Only layer A of core 0 is bound (the program's piano layers 2 and 3 are not separated); a program with both layers enabled binds layer A's sound only.
- A second key in one run blocks in the voice status poll ([piano-sound-load](piano-sound-load.md) s7); the runs play one key.
- `install_dsp_ready` models the DSP serial link as always ready (ASSUMED, unchanged).

## Reproduce

```bash
C=target/fw_unpacked; B=".reference/Nord Stage 4 Factory Restore RevK v1.66.ns4b"
python tools/ns4_dsp_raw.py target/DSP/dsp_image_4_SMP.bin --sram target/oracle/smp.bin
cargo build --release -p nord-sharc --bin sharc-live
uv run --with unicorn --with capstone python tools/arm_dsp_live.py $C/bank0_CODE.bin $C/bank0_SRAM.bin \
    --eeprom target/oled/eeprom.bin --program "factory_programs/Program/Bank A/Astoria Grand.ns4p" \
    --live target/release/sharc-live --poll-advance 64 --chip "PNO:1=pno=target/PNO/core1_sram.bin" \
    --chip "SMP:0=smp=target/oracle/smp.bin" --piano-bind "$B" --key-ticks 400 --log smplog --out astoria.json
```

`phases.key.arm_messages.SMP0p1` is 117, `final.SMP0.lane_peak` 0.0, `final.SMP0.errors` empty. About 90 s of CPU per program. Table of the four runs: [data/piano-chain-tap-smp.csv](data/piano-chain-tap-smp.csv).

## Implications

- Blockers 1 and 2 of OQ41 are closed; the voice start of a piano key is now observable on a live SMP chip (record words, gains, descriptors), which is a *voice-start tap* for the velocity law without audio.
- The ranking cannot get a piano chain column until the page refill (blocker 3) gives a non-zero bus.

## Follow-ups

1. SMP external image for the EP chain (s4): serve the bound `.npno`, log the TCB addresses, decode the descriptor words (b), (c).
2. Run the 155 piano programs with `--piano-bind` once audio exists; add the column and drop `piano_no_chain_tap` per program.
3. Separate piano layers 2 and 3 (`select_sound` binds layer 0 of core 0 only).
4. Check the ring half choice (`live_smp::ring_half`) on the first non-zero ring.
5. Fix `ns4_inspect._decode_piano_layer` (2 bit shift against `piano_preset.rs`, test pins the old layout).

## Update 2026-11-21 (D191)

Follow-up 1 is done: the descriptor law and the flash feed are in [smp-page-refill](smp-page-refill.md). The rings now hold the file words, but the voice phase freezes after the first refill, so follow-ups 2 and 4 wait for the per-voice gate.

## Update 2026-11-21 (D205)

Follow-ups 2 and 4 are done for the pilot programs: see [smp-voice-pitch](smp-voice-pitch.md). The tap carries a piano signal once the ARM pairs are paced (`--fifo-pace 1`); follow-up 2 (census) is done (D216: 152 of 155 programs, [program-ranking](program-ranking.md) s3.3a); follow-up 4 (ring half): the half the pointer holds on entry is right, one frame per interrupt is written at `+0`, the other half and the second slot stay 0.

