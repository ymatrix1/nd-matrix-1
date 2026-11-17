# Research: ARM to DSP message capture over all 384 factory programs, and the sample pitch entry `e` (D124)

- **Checked:** 2026-11-17
- **Sources (local, never committed):** OS v1.66 `target/fw_unpacked/bank0_CODE.bin` and `bank0_SRAM.bin`, the factory restore EEPROM image `target/oled/eeprom.bin`, the 384 programs of `factory_programs/Program/`, the sample file `Samp Lib/Accordion_Harm/Carina Accordion 1_PS 4.1.nsmp4` of `Nord Stage 4 Factory Restore RevK v1.66.ns4b` (first 4 MB, bound as a stand-in sample). Tools: `tools/arm_dsp_capture.py`, `tools/arm_dsp_summary.py` (new, `tools/tests/test_arm_dsp_*.py`), built on `arm_program_load.py`, `arm_org_stream.py`, `arm_keydown.py`, `arm_sy_voice.py`, `arm_sample_probe.py`. Background: [sy-pitch-entry-writer](sy-pitch-entry-writer.md), [synth-sample-kernel-rule](synth-sample-kernel-rule.md), [org-system-run](org-system-run.md), [arm-param-messages](arm-param-messages.md).
- **Question:** Does the ARM send the SY sample pitch entry `e` (`DM(0x9354A + index)`) when a program with a SAMPLE layer is recalled and a key is played, and if not, what does it send instead? What is the whole ARM to DSP stream of the 384 factory programs, by engine?

Tags follow `.agents/rules/provenance-in-code.md`: **VERIFIED** = reproduced by the command in "Reproduce", **EMPIRICAL** = measured in this harness, **ASSUMED** = reading or placeholder.

## 1. Short answer

| Question | Result | Tag |
|---|---|---|
| **Does the ARM write an `e`-shaped value?** | **No.** In 119 of 119 programs with an active SAMPLE synth layer (113 in scene I, 6 only in scene II) not one ARM to DSP word of any chip lands in `DM(0x93500..0x935A0)` (0 hits), and not one 16-bit half of a key-phase word is within 2 of the modelled entry `pack_ratio(word / 0.05)` of the pitch word the ARM sent in that phase (0 hits, 60 programs / 149 played keys reach the voice path, section 3). The writer of `e` is **not an ARM message in any path the harness runs** (recall, stand-in sample bound with the real loader, key down); the hunt does not end here. | VERIFIED (run, `arm_dsp_summary.py` e check) |
| What does the ARM send to SY for a sample note? | Per played voice 17 words, all in `DM(0x921A6 .. 0x9252C)`: state reset / prepare (`0x92414` = `0x1C0794`, `0x1C07CD`), pitch word (`0x921A6`, float), two gains (`0x3F128F5C`, `0xBEDAE148`), a kernel base (`0x9243B`, `0xE3400 ..`), three (count, offset) record pairs plus one field (`0x9240A .. 0x92413`) and `0x92432 = 0xBF800000`. 76 distinct SY words over all voiced captures, none above `0x9252C` | VERIFIED (run) |
| What does the recall alone send to SY and SMP? | **Nothing sample specific.** SY gets **0 messages** during program load in all 384 programs (also in the 119 sample ones); SMP gets the same 1,516 path 1 words in every program (618 of 686 words constant, 68 vary in a few effect programs: `0x94086..` in `Rotor EP` / `Rotor Clav`, `0xB207A..` e.g. in `70s Phase Mk I`). The real sample loader `0xC01D710C` plus the layer bind send **0 DSP messages** (one run, `Acoustic Guitar`, scratch probe). | VERIFIED (summary run), EMPIRICAL (loader probe) |
| Is the sample id sent? | **No.** The 32-bit `sample_id` of the layer (layer bits `+26..+58`, 108 distinct ids in the 119 programs, a hash: not CRC-32, Adler, FNV-1/1a, djb2, sdbm, Jenkins of the file name, stem or header name) appears as a full word in **no** message of any program (0 of 119) | EMPIRICAL (scratch scans, not in the tools) |
| Descriptor, flash address or table sent instead? | **None.** No SY or SMP message carries a sample address, length or flash page; the (count, offset) pairs `0x9240A..0x9240F` are per-zone values (offsets are multiples of `0x20`, [synth-sample-kernel-rule](synth-sample-kernel-rule.md) s14), the kernel base `0x9243B` is the DM address of the 12-tap kernel (`0xE3400 + 0xC00 n`). Where the sample data and `e` come from is therefore **not in the ARM message stream**: it has to be a DSP side fill (SPI flash / external-port DMA) that the SY image starts itself, or a transfer the harness does not run. | VERIFIED (negative), the DMA reading is ASSUMED |
| Bulk stores | 23 of 384 programs send 3 K to 79 K path 2 stores outside the DSP core window (`0x200xxxxx`, store pc `0xC01C0B70`, to ORG and PNO core 1; 564,736 words in total); 21 of the 23 have no active SAMPLE layer, the 2 SAMPLE ones also have a Wavetable layer. 86 more programs send exactly 300 FX stores outside the window (pc `0xC002B77C`). Neither is the sample path | EMPIRICAL |

## 2. Method and what the harness does not model

`tools/arm_dsp_capture.py` boots the full machine (`arm_screens.Session`) with the program in the Store record (`arm_program_load.build_image`, the firmware recalls it itself), attaches the board-present flag and the UART2 MIDI model, installs the DSP ports and then logs every DSP message in these phases (1 ms ticks, 200,000 ARM instructions each, [org-system-run](org-system-run.md)): `load` 800, `idle` 100, then for the keys 36, 60 and 84 (velocity 100) `k<N>` 40 ticks on and `k<N>_off` 40 ticks off, each key from the same saved state (`Session.state_bytes` / `reset`, because a second note-on in one run finds the first voice held and sends nothing). One worker process per program (about 70 s, 10 workers, 384 of 384 finished, 0 failed, resumable by file). Rows are `phase tick chip path core dm w2 pc`, gzip per program under `target/arm_dsp_all/` (git-ignored, about 15,300 messages per program).

| Part | State |
|---|---|
| Event queue drain | The harness runs no consumer task, so the synth voice events (`0xC01E00AC`) stay queued. The capture drains the queue after every tick (`arm_sy_voice.pump`) and sets the DSP status bit the voice update waits for (`ACK_PCS`, the same ASSUMED "SY voice ready" bit as `play_note`). Without it the SY chip receives nothing even for a bound sample. |
| Sample library | **Not modelled.** The recall sets the SAMPLES flag `[layer + 0x4BC] = 1` and stores the sample id hash, but no sample is loaded (slot list empty, `synth_gate` closed): without more the synth voice event ends before any SY message (`80s Sci-Fi` analog control and `Acoustic Guitar` both give 0 SY messages). The capture therefore **binds one stand-in sample** (`Carina Accordion 1`, library entry 1, real loader `0xC01D710C` and bind `0xC01E5978`, [synth-sample-kernel-rule](synth-sample-kernel-rule.md) s11) to every layer whose recall set the flag. The file the program really names is **unknown**: the hash to file map is not found (section 5). So per-program SY values (record fields, kernel base) describe the Accordion zones, not the program's own sample; only the **shape and addresses** of the stream, and the absence of `e`, carry over. |
| Voice path | The voice update ran for 60 of the 119 programs (30 programs with all three keys, 29 with two, 1 with one; 149 voices). In the other 59 the voice event `0xC01EC690 -> 0xC01E72C4` is entered but stops at the gate before the voice update (`String Quintet`: slot bound, the allocator counters `0xC05B0C78 + 0x78 / 0x7C / 0x80` are 0, but they are 0 in the voiced `Acoustic Guitar` too); the cause (polyphony limits, zone, MIDI channel) is **not diagnosed**. 6 of these are scene II only (scene I is recalled). The e check below covers all 119 programs' messages, but only the voiced 60 can show an `e`. |
| MIDI channel | Notes go out on channel 1 only. A sweep of all 16 channels on `80s Sci-Fi` sends no SY word (analog layers use the ORG / PNO path). |
| Messages | `Message.dm_word` is the word address in the receiving core (path 2) or the DM word (path 1); chips are the ASSUMED `COMM_NAMES` order `FX, PNO, ORG, SY, SMP` ([arm-param-messages](arm-param-messages.md) s3). |

## 3. Evidence for the `e` verdict

`python tools/arm_dsp_summary.py target/arm_dsp_all factory_programs/Program --out docs/research/data` (data: [arm-dsp-capture-e-check.json](data/arm-dsp-capture-e-check.json), one row per sample program):

| Check | Result |
|---|---|
| Programs with an active SAMPLE synth layer (scene I or II, layer mask 0 = layer A) | 119 (113 + 6), 134 programs have a SAMPLE layer of any kind |
| Messages (any chip, any phase) with `0x93500 <= dm < 0x935A0` | **0** in all 384 programs |
| Key-phase words with a 16-bit half within 2 of `modelled_entry(pitch word of that phase)` | **0** (the model entry is `pack_ratio(word / 0.05)`, D65, [sy-pitch-entry-writer](sy-pitch-entry-writer.md)) |
| SY messages outside the 119 sample programs | 0 over every phase (assert in the scan: the 265 other programs send no SY word) |
| Distinct SY DM words in all voiced key phases | 76, range `0x921A6 .. 0x9252C` (voice pair block, record, gains, kernel base) |
| Sample load plus bind, measured | 0 DSP messages (one run, `Acoustic Guitar`, messages cleared before the loader call) |

The words in the table of section 1 vary as follows ([arm-dsp-capture-e-check.json](data/arm-dsp-capture-e-check.json), `sy_key_words`; 60 voiced programs): the pitch word (`0x921A6`, `0x921A7` for the second voice) and the kernel base `0x9243B` vary with the key; the record pairs `0x9240A..0x92413` vary with the key and with the program (octave shift and transpose enter the effective key); the gains `0x92420 / 0x92421` and `0x92432` are constant; `0x92414` repeats the reset and prepare values. Further voices repeat the block at a stride of `0x38` words (`0x92442 .. 0x92473`, `0x9247A ..`, ..).

## 4. The whole stream by engine

Phase `load` (800 ticks after the boot), bulk stores excluded (they are counted apart, section 1). Messages per program; words are DM words of the target core. "constant" = same final value in every program, "varying" = more than one value across the programs. Data: [arm-dsp-capture-targets.json](data/arm-dsp-capture-targets.json), per program [arm-dsp-capture-programs.csv](data/arm-dsp-capture-programs.csv) (class and counts per target).

| Engine | Target | Load msgs min / median / max | Key msgs per 3 keys, max | Distinct words | Written by every program | Constant | Varying | Program specific |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| Organ | ORG core 1 | 876 / 912 / 2,772 | 26 | 4,043 | 876 | 557 | 1,878 | 3,167 |
| Organ | ORG core 2 | 876 / 912 / 2,772 | 26 | 4,094 | 876 | 557 | 1,878 | 3,218 |
| Piano (and organ models, D119) | PNO core 1 | 6,865 / 7,248 / 7,406 | 327 | 4,827 | 3,767 | 3,347 | 591 | 1,060 |
| Piano | PNO core 2 | 876 / 912 / 2,772 | 35 | 3,997 | 876 | 557 | 1,878 | 3,121 |
| FX | FX core 1 | 1,840 / 1,854 / 2,237 | 0 | 2,577 | 1,810 | 460 | 1,797 | 767 |
| FX | FX core 2 | 1,597 / 1,905 / 1,905 | 0 | 1,895 | 1,589 | 1,589 | 0 | 306 |
| Synth samples | SY (path 1) | 0 / 0 / 0 | 255 | 0 in load, 76 in key phases | n/a | n/a | n/a | n/a |
| Synth (SMP DSP) | SMP (path 1) | 1,516 / 1,516 / 1,516 | 0 | 686 | 686 | 618 | 68 | 0 |

- **Message ids.** There is no separate id: a message is `(DM word, value)` to one core. ORG core 1, ORG core 2 and PNO core 2 receive the same 876-word strip set (the 876 common words are equal in count, 912 median), PNO core 1 additionally gets the note and organ model words, FX core 1 the effect parameters, FX core 2 a fixed 1,589-word set.
- **Which words vary with the program.** The 1,878 varying words of the ORG / PNO strips (535 DM ranges) are the layer settings (levels, filter, envelope, strips of the active layers); the 591 varying PNO core 1 words are the organ / piano layer state; FX core 1's 1,797 are the effect parameters. FX core 2 and the 618 SMP words never change with the program.
- **Routing.** The section and layer enables stay ARM side: they select which zone bytes and which strips are sent, there is no routing message group of their own. The SY chip is silent during load (no sample is loaded), so synth sample routing only shows in the key phases.
- **Key messages.** 0 for the three FX targets and SMP; ORG up to 26, PNO core 2 up to 35, PNO core 1 up to 327 for the three keys (notes land in the PNO image, [org-system-run](org-system-run.md) s2); SY 17 words per voice (section 3).
- **Sample programs against the others.** The 113 scene I sample programs have no DM word that at least 80 % of them send and under 5 % of the analog synth programs (165) and of the programs without synth (106) send, for any target: the recall carries no sample specific word (EMPIRICAL, scratch scan over the capture files, not in the tools).
- **Bulk stores.** The 109 programs with stores outside the core window (section 1) are excluded from the table and counted in the `bulk` column of the programs CSV; they would add up to 79,305 words to one program (`Dance Lead`).

## 5. Implications

- The **writer of `e` is not an ARM message** in any of the 384 recalls and 149 played voices. Combined with [sy-pitch-entry-writer](sy-pitch-entry-writer.md) (no DSP store, no ARM port, SMP code does not reference the window) the entry has to come from a transfer the ARM does not describe word by word: an SPI flash or external-port DMA the SY image runs itself, or the SMP DSP. D111 / D112 already pointed at the real sample stream; this run adds that the ARM sends **no address or descriptor** that such a DMA could be parametrised by (the pair offsets are per-zone values, the id hash never leaves the ARM).
- A flash / SPI DMA model therefore needs, as input, the sample **data** location, which the ARM message stream does not give: it must come from the DSP image (a fixed flash layout the SY ISR reads, e.g. opcode 9 `0x1C03AA`) or from a table the ARM writes to flash storage the DSP reads, not through the DSP port.
- `--sample-pitch fw` stays opt-in; no engine change, no default flip (D22), no re-baseline.
- The synth voice path needs the sample library to run at all; a harness that binds the program's own sample needs the hash to file map (unknown) or the real directory.

## 6. Follow-ups

1. Resolve the sample id hash to a library file (not CRC-32, Adler-32, FNV, djb2, sdbm, Jenkins of file name, stem or header name; the ARM computes or reads it: find the lookup in the sample loader `0xC01D710C` path or the directory build) so the capture can bind the program's own sample.
2. ~~Diagnose why 59 of 119 sample programs stop at the voice gate~~ **answered, D143 ([sample-voice-gate](sample-voice-gate.md)):** the harness serves SPI1 `0x01E12040` as RAM and the scheduler `0xC01DEAFC` defers the voice event; fixed in the capture (default `--spi-neutral`, 110 of 119 voiced on key 60). Capture the other keys with it.
3. Look at what the **real library load** does on the ARM side (file reads of the `.nsmp4`, any SPI or SDRAM write of sample data, `0xC01C0B70`-like bulk stores) with the program's own sample: the bulk store path of this run is the only ARM to DSP data transfer seen.
4. SPI flash / external-port DMA model (separate task): input is the SY image side (opcode 9, `0x1C03AA`), not the ARM stream.
5. Add the key phases for programs with splits (the 59 unvoiced ones may only sound in another key range).

Note (D143): the counts above (60 of 119 voiced, 59 stopped) were taken before `--spi-neutral` became the default; see [sample-voice-gate](sample-voice-gate.md).

## Reproduce

```bash
C=target/fw_unpacked        # firmware, never committed
cd tools && uv run --with unicorn python arm_dsp_capture.py batch ../$C/bank0_CODE.bin ../$C/bank0_SRAM.bin \
  --eeprom ../target/oled/eeprom.bin --out ../target/arm_dsp_all --workers 10 \
  --ns4b "../.reference/Nord Stage 4 Factory Restore RevK v1.66.ns4b" ../factory_programs/Program
# about 70 minutes with 10 workers (55 to 130 s per program); resumable: finished programs are skipped
uv run --with unicorn python arm_dsp_summary.py ../target/arm_dsp_all ../factory_programs/Program --out ../docs/research/data
uv run pytest tools/tests/test_arm_dsp_capture.py tools/tests/test_arm_dsp_summary.py
```

The last line of the summary prints `e check: 119 sample programs, 0 window hits, 0 e-shaped hits`. One program: `arm_dsp_capture.py one ... "<program>.ns4p"` (add `--ns4b` to bind the stand-in sample). Quick controls used here (no files committed): the analog program `80s Sci-Fi` and `Acoustic Guitar` without `--ns4b` give 0 SY messages in every phase; with `--ns4b` `Acoustic Guitar` gives 17 SY words per key, all in `0x921A6..0x9252C`.
