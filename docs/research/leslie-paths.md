# Research: B3 Leslie gate, wider paths: 7 programs, 20,762 edit phases, live link (not found in the paths run; D155)

- **Checked:** 2026-11-17
- **Sources (local, never committed):** OS v1.66 ARM image `target/fw_unpacked/bank0_CODE.bin` and the settled ARM harness; the five DSP images `target/DSP/dsp_image_*.bin` and their unpacked cores (`tools/ns4_dsp_ldr.py --sram`; the SY1SY2 and SMP images do not parse with this loader and are covered by a raw byte scan only); the live link `tools/arm_dsp_live.py` with `sharc-live` (PNO cores 1 and 2, ORG cores 1 and 2); `sharc-disasm` listings. New: `tools/arm_leslie_paths.py` (+ `tools/tests/test_arm_leslie_paths.py`). Numbers and addresses only; no firmware, sample or audio content is printed or committed. Data: [data/leslie-paths.json](data/leslie-paths.json). Builds on [leslie-gate](leslie-gate.md) (D148), [leslie-locate](leslie-locate.md) (D139), [b3-leslie-fx-chip](b3-leslie-fx-chip.md) (D135), [b3-leslie-pno-lane](b3-leslie-pno-lane.md) (D126), [arm-dsp-live-link](arm-dsp-live-link.md) (D138), [arm-dsp-live-link-synth](arm-dsp-live-link-synth.md) (D144).
- **Question:** After D148, is there any ARM path (more programs, FX core and System edits, run-time loaded images) or live DSP chip (PNO 1 / 2, ORG 1 / 2) that names, loads or executes the B3 Leslie stage `0x1256AD` (or `0x1237F8`, `0x1257B2`)? If so, how does the stage compare with the native Leslie?

Tags follow `.agents/rules/provenance-in-code.md`: **VERIFIED** = reproduced by a command under Reproduce, **EMPIRICAL** = measured on this host, **ASSUMED** = reading without a source. Everything below is limited to "the paths run".

## 1. Short answer

**Not found in the paths run.** No ARM message, DMA, code store or image load names `0x1256AD`, `0x1237F8` or `0x1257B2` in seven recalls of 6,000 ticks plus 2,966 edit phases each, and the live PNO core 1 does not execute `0x1256AD` in 2,441 DSP passes. Two corrections to earlier reading come out of this, and the remaining gate hypotheses are listed with counts in section 5. The native Leslie is not changed (no oracle run of the stage in a linked chain exists, so no measured gap; D148 numbers stand).

| # | Check | Result | Tag |
|---|---|---|---|
| 1 | Literal scan of every ARM to DSP message (all chips, cores, paths) of 7 recalls (6,000 ticks) plus idle (200 ticks) plus 2,966 edit phases (40 ticks each) per program | **0 hits** for `0x1256AD`, `0x1257B2`, `0x1237F8`, as value or as code record `0x28000000 + 4 r`, in 7 x 24,247 .. 24,597 messages (Rotor Clav 23,458). Positive controls in the same scan: the B3 processor `0x121E2B` 1 .. 2 hits, Amp words `0x1250CF` 21 .. 23, `0x125371` 8, `0x1235AA` 13 .. 15, `0x1236F8` 13 .. 15 per program | VERIFIED (`tools/arm_leslie_paths.py`, [data](data/leslie-paths.json)) |
| 2 | Run-time loaded code or images | **0 DSP L2 / code stores** (path 2 byte address `0x20000000..0x28000000`) in all 7 programs; the DSP image download `0xC0241C58` is called 1 time and the SPI block download `0xC0240548` 4 times, **all in the boot, 0 after the recall starts**; the bulk table sender `0xC001C6A0` 2 calls, both in the boot (the engine constructor). So no image or table that could carry the routine is loaded at run time | VERIFIED (code hooks, same run) |
| 3 | Programs | `B3 Chords n Solo` (rotary on, slow, drive 2.44), `B3 Shade` (on, slow, 3.7), `B3 Dual Samba` (on, **fast**, 0), `Solo n Walk B3` (on, **stop**, 4.25), `B3 Old Forest` (**off**, slow), `Vintage Vox J3` (off, fast, 3.15), `Rotor Clav` (a piano layer with rotary, on, stop, 2.52). Same result for all 7 | VERIFIED (`tools/ns4_inspect.py --decode`, the same runs) |
| 4 | FX cores edited | 232 type edit phases per program on the four FX units 3 and 9 (organ), 4 (piano), 0 (synth): amp type 0..9, `kFx1Type`, `kFx2Type`, `kRvbType`, `kDlyFbFxType` 0..11. They send 4,622 messages to **FX core 1** (4,566 for Rotor Clav) and 3,131 .. 3,290 to PNO core 1; **0 messages to FX core 2, ORG, SY, SMP**. None carries a literal (section 1 row 1) | VERIFIED |
| 5 | Leslie / rotary ids | 160 phases per program on every store that owns the id (organ FX units 3 and 9 own the `Edt_OrgCom_*Leslie*` ids and `Fnc_kRotorSpeed` 0..5; six units own `Edt_Sys_kLeslieHornSpeed / HornAcc / RotorSpeed / RotorAcc / Balance`). Result: **11 distinct words**, all PNO core 1: the three rotor speed words `0xB2F25`, `0xB2F26`, `0xB2FA1` (27 each), the drive words, and the Amp stage words `0xB041D`, `0xB0424`, `0xB0425`, `0xB047E`, `0xB047F`, `0xB0481..0xB0483` (1 each). Rotor Clav: 8 words, no speed word. No routine pointer | VERIFIED |
| 6 | System routing ids | `Edt_Sys_kAudioRoutingDstLeslie` (`0x4e`), `Edt_Com_kAudioRoutingBusLeslie` (`0x59`) and the other 17 of the 19 `AudioRouting` ids (2,574 phases per program, each id on all 19 stores): **no store of the dispatcher owns them** (the 3 `ext` layers own 32 `Edt_Sys` MIDI and `Edt_Ext` ids, none of these; 0 value cells in the 10 layer clients and 12 FX units). Dispatched to all 19 stores anyway: the 1,221 messages are the 3 periodic rotor speed words (407 sets, one set per about 253 ticks, the rate of the idle phase); **0 words are caused by a routing id** (Rotor Clav: 0 messages). So the routing ids are not handled by the ARM edit dispatcher in this firmware path | VERIFIED (dispatch run) for "no cell and no message"; where they are handled stays open |
| 7 | Static: who can run the Leslie words | The words `DM(0xB2F21 .. 0xB2F31)`, `DM(0xB2FA1)` are referenced by **9 instructions in PNO core 1 only** (`0x24AD70..0x24ADAA` the stage, `0x24B532..0x24B54E` its callee row); 0 references in FX cores 1 and 2, PNO core 2, ORG cores 1 and 2. A raw byte scan of the instruction `R1 = DM(0xB2F25)` over the five DSP image files finds 1 occurrence in the PNO image and 0 in FX, ORG, SY1SY2, SMP | VERIFIED (`sharc-disasm` listings of 80,915 lines; raw scan) |
| 8 | Live link: which chip executes `0x1256AD` | `B3 Chords n Solo` and `B3 Dual Samba`, 2,441 passes each (800 load, 100 idle, 1,500 key, 300 off ticks), probes `0x1256AD`, `0x1237F8`, `0x1257B2` on PNO 1, PNO 2, ORG 1, ORG 2. **PNO 1: 0 hits for all three**, control `0x121E2B`: 425 of 723 load passes, then every pass (90, 1,357, 271). **PNO 2 and ORG 2 hit `0x1256AD` in 516, 90, 1,357, 271 passes** (every pass after the install): but there the pc is inside the synth filter stage `0x12566E` (`F12 = F4 * F4`, the reciprocal square root step), not the Leslie, see correction 1. ORG 1 ran 1 pass (not usable). `0x1237F8` and `0x1257B2` 0 on all four | VERIFIED (live runs, [data](data/leslie-paths.json)) |

## 2. Corrections to earlier readings

1. **The same pc number is a different routine on core 2.** In the PNO core 2 and ORG core 2 images, `0x1256AD` (byte `0x24AD5A`) is the instruction `F12 = F4 * F4` inside the filter stage whose entry is `0x12566E` (byte `0x24ACDC`; D121 stage calls, 16 per pass). In the PNO core 1 image the same pc is the first word of the Leslie stage (`DM(0xB4CCC) = I0`, then reads of `DM(0xB2F23 / 0xB2F25)`). A probe on `0x1256AD` on a core 2 chip reports hits that are the filter stage, so such a probe means nothing for the Leslie. Probes must be placed on the PNO core 1 image only. VERIFIED (`sharc-disasm target/.../core2_sram.bin 0x24AD54 0x24AD70` against `core1_sram.bin 0x24AD40 0x24ADA0`; PNO 2 and ORG 2 listings are byte-identical in that range).
2. **`0x1257B2` is not a Leslie candidate.** It is the raw synth filter type 3 stage (D121, 61 of 988 filter-on factory layers; in the strip word 1 table of [pno-module-chain](pno-module-chain.md)); its one literal in the ARM image is most likely the stage table entry of D121 (ASSUMED reading). In the 7 recalls it is sent 0 times (no synth filter type 3 in these programs); in the live run it does not execute (0 on four chips). It stays an open filter item (D121), not a Leslie one.

## 3. What the numbers say

- **Not an ARM-sent install in the paths run.** The Leslie stage lives only in the PNO core 1 image (row 7); the ARM never sends its address (row 1), loads no code or image after the boot (row 2), and the live PNO core 1 executes it in 0 of 2,441 passes while the B3 processor from the same recall runs every pass (row 8).
- **Edits of the three groups produce no gate** (rows 4 to 6): FX types and the Amp family write the `0xB0476` stage set (the Amp module, D148 row 2) and the FX core 1 words; the Leslie ids write the three speed words and drive; the routing ids write nothing.
- The boot installs `0x1237F8` in the Leslie callee row `DM(0xB2F29)` (D135 boot log; the ARM image has the code-record literal once). The recall does not repeat it (row 1: 0 hits for `0x1237F8` in the load phase). So the callee row is set at boot and the caller `0x1256AD` is the part with no install.
- The unchanged hardware fact (D148 row 4): the clip has horn lines at 0.773 and about 7.0 Hz. A rotary runs on the hardware; this harness does not make it run.

## 4. Oracle and native comparison

Not done: the stage never executes in a linked chain, so there is nothing new to measure beyond the direct drive of D148 (`leslie_probe`: rates follow the speed words, +6.4 dB at 2 kHz, -11.6 dB at 5 kHz, crossover not separable). No gap against the native rotary (`organ/rotary.rs`) is clear enough to port; no opt-in was added and no `nord-core` file changed. The failing-first test rule for a native fix is not triggered.

## 5. Remaining hypotheses (with what was and was not covered)

| # | Hypothesis | Covered by | Not covered |
|---|---|---|---|
| H1 | The install is a PNO core 1 chain-head write (`DM(0xC2F48 / 0xC2F82 / 0xC2FBC / 0xC2FD6`, 4 heads, each read once in the listing, 0 writes by name) by the DSP itself from a table it fetches by DMA | the ARM sends no head word in 7 x 24,000 messages; 0 L2 stores | DSP side DMA from flash or SDRAM: no model (D141: the SY has no SPI flash; PNO unchecked); the live link has SPI1 as RAM |
| H2 | A part command consumer on the ARM (list `[part + 0xC8]`, node handler `0xC010E774`) that the harness never drains | the harness runs the tick tasks and `arm_sy_voice.pump` only; 2,966 edit phases per program through the dispatcher | the consumer itself (D139 follow-up 1); a DSP read-back handshake answered by the stub `0xC0066E64` (0) |
| H3 | A user gesture that is not an edit id: the organ section on/off, the layer enable masks (`enable_ii_mask`), the rotor speed pedal / switch UART events, the preset key | `Fnc_kRotorSpeed` 0..5 dispatched on both organ units (rotor speed words only) | MIDI / UART events for the speed switch and pedal, the organ on/off and part enable edits, a key press while the rotary is switched on after the recall (the key phases ran only in the live link, 1,500 ticks, rotary on) |
| H4 | The hardware chain runs a different routine for the Leslie than `0x1256AD` (for example on an image not listed: SY1SY2, SMP) | the raw scan finds the Leslie word read in PNO only; FX, ORG 0 | SY1SY2 and SMP unpacked listings (the loader does not parse them); the hardware clip is not matched to any routine |
| H5 | Longer ARM time: the install needs more than 0.25 s (6,000 ticks) | 7 programs at 6,000 ticks (D148: one program) | seconds of ARM time and a program change after a long idle |
| H6 | The Leslie output is not a DSP stage chain element but a routing target (D148 ASSUMED): the ids `0x4e`, `0x59` | they have no value cell and no message (row 6) | the system routing store (outside the dispatcher): who handles them, in the ARM image 0 literals found in the id tables beyond the names |

## 6. Implications

1. Add no gate: the paths in section 5 are the open items. Rows 1 to 8 are repeatable with one command per program.
2. Do not use a core 2 chip to probe the Leslie (correction 1); future probes name the image.
3. D148 stays as written for the stage and the clip numbers; this note extends the negative to 7 programs, FX type edits, System / Common Leslie ids, routing ids, and the live PNO 1 and ORG 1 / 2 and PNO 2 chips.
4. No default, golden, `render_hash` or native Leslie change; `--leslie firmware` stays opt-in (D22, D63).

## 7. Follow-ups

1. H2: run the part command consumer (list `[part + 0xC8]`, handler `0xC010E774`) with the read-back answered by the oracle; watch row 6 and the heads in the live link.
2. H3: send the organ on/off and the rotor speed switch as UART / MIDI events in the live link (`arm_keydown.Uart2`) and read PNO core 1 for the first execution of `0x1256AD`.
3. H4: extract the SY1SY2 and SMP images (a loader for their section format) and scan for the Leslie word reads.
4. H5: one program with 60,000 ticks (2.5 s) before the key phase.
5. A second hardware clip with a held fast speed (D148 follow-up 3) is still the only way to separate the drum / horn model from the single clip.

## Reproduce

```bash
C=target/fw_unpacked; M=<main checkout>   # firmware stays local
# 7 programs, about 60 min CPU each (2,966 edit phases); writes target/leslie_paths/*.json and *.txt.gz
for p in "Bank A/B3 Chords n Solo" "Bank A/B3 Shade" "Bank B/B3 Dual Samba" "Bank B/Solo n Walk B3" \
         "Bank A/B3 Old Forest" "Bank A/Vintage Vox J3" "Bank D/Rotor Clav"; do
  uv run --with unicorn python -I tools/arm_leslie_paths.py $C/bank0_CODE.bin $C/bank0_SRAM.bin \
      --eeprom target/oled/eeprom.bin --out target/leslie_paths "factory_programs/Program/$p.ns4p"
done
# live link, 4 chips with probes 0x1256AD,0x1237F8,0x1257B2,0x121E2B (about 6 min)
cargo build --release -p nord-sharc --bin sharc-live
uv run python tools/ns4_dsp_ldr.py target/DSP/dsp_image_2_ORG.bin --sram target/leslie_paths/ORG
uv run --with unicorn --with capstone python -u tools/arm_dsp_live.py $C/bank0_CODE.bin $C/bank0_SRAM.bin \
  --eeprom target/oled/eeprom.bin --program "factory_programs/Program/Bank A/B3 Chords n Solo.ns4p" \
  --chip "PNO:1=pno=target/PNO/core1_sram.bin=0xC1F89:4:2=0x1256AD,0x1237F8,0x1257B2,0x121E2B" \
  --chip "PNO:2=org=target/PNO/core2_sram.bin=0xC1F89:4:2=0x1256AD,0x1237F8,0x1257B2,0x121E2B" \
  --chip "ORG:1=org=target/leslie_paths/ORG/core1_sram.bin=0xC1F89:4:2=0x1256AD,0x1237F8,0x1257B2,0x121E2B" \
  --chip "ORG:2=org=target/leslie_paths/ORG/core2_sram.bin=0xC1F89:4:2=0x1256AD,0x1237F8,0x1257B2,0x121E2B" \
  --live target/release/sharc-live --out live.json
# correction 1 (byte address = 2 x pc): filter stage on core 2, Leslie on core 1
target/release/sharc-disasm target/leslie_paths/ORG/core2_sram.bin 0x24AD54 0x24AD70
target/release/sharc-disasm target/PNO/core1_sram.bin 0x24AD40 0x24ADA0
```
