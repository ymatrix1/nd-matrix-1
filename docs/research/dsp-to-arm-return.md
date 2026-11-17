# Research: the DSP to ARM return path, what the ARM reads back and what an answer changes (D147)

- **Checked:** 2026-11-17
- **Sources (local, never committed):** OS v1.66 `target/fw_unpacked/bank0_CODE.bin` and `bank0_SRAM.bin`, factory restore EEPROM `target/oled/eeprom.bin`, factory programs `Acoustic Guitar`, `String Quintet`, `Grand Pad`, `4 Osc Funk Bass`, one stand-in sample bound by the real loader (as [arm-dsp-capture](arm-dsp-capture.md)), SY image `target/oracle/sy.bin`, flat DSP images `target/PNO` and `ns4_dsp_ldr.py --sram` of the ORG image. Disassembly with `capstone` (addresses and numbers only). Background: [arm-dsp-live-link](arm-dsp-live-link.md) (D138), [sample-voice-gate](sample-voice-gate.md) (D143), [dsp-flash-dma-model](dsp-flash-dma-model.md) (D141), [sy-smp-audio](sy-smp-audio.md), [dsp-rx-protocol](dsp-rx-protocol.md). Tools (new): `tools/arm_dsp_return.py`, `tools/arm_dsp_return_run.py`, `--return-path` of `tools/arm_dsp_live.py`.
- **Question:** Which DSP to ARM signals does the firmware actually read, which code reads them, and what changes in the ARM when the DSP answers them instead of the harness stubs (SPI1 as RAM, `DspAck` by hook)?

Tags follow `.agents/rules/provenance-in-code.md`: **VERIFIED** = reproduced by a run or listing named here, **EMPIRICAL** = measured in this harness, **ASSUMED** = reading.

## 1. Short answer

| Question | Result | Tag |
|---|---|---|
| How does the ARM read the DSP? | Only by **SPI reads of DSP data memory**: request `(chip, DSP address, word count)`, answer = the DSP words. Three routines: `0xC00119A4` (4 words), `0xC00117E8` (2 words), `0xC0066E64` (any count, comm object argument). The device behind `0xC00119A4` that D143 left open is **the DSP SPI slave** (address words `0x28307770` / `0x28B07770` are DM word `0xC1DDC` of core 1 / core 2 in the `0x28000000` alias). | VERIFIED (listing, runs) |
| Which signals are read in a note? | (1) **SY idle voice mask** `DM(0x92196..7)`, voice `k` = bit `k`: the voice update `0xC01E00AC` waits for bit 0 of word 0 (voice 0) before it sends the prepare state. (2) **Three note-on peeks** of `DM(0xC1DDC..F)` on PNO core 2, ORG core 1, ORG core 2 (table refill `0xC01B1840`), gate `0xC01DEAFC`. No other read was found in the paths run (500 ticks, 3 notes). | VERIFIED (run) |
| Sample ring refill request? | **Not found in the paths run.** The SY refills its ring by its own external-port DMA (D141); no ARM read of ring state, read pointer or refill flag occurred in the 2 SAMPLE programs run (reads are only the two items above). Other `0xC0066E64` sites are not decoded, so this is not proof of absence. | VERIFIED (negative, only for the paths run) |
| Do the answers change the ARM? | **No change in the cases run.** With the return path the 2 SAMPLE-flagged programs (Acoustic Guitar, String Quintet) send the same SY words as the stub (17 SY messages, 1 prepare, key-down delay 1 tick); the gate reads 0 (images and shadow are 0, no live chip answered) which equals the D143 neutral read. The harness `DspAck` hook is replaced by a real read of the SY mask and the same bit is found set. | EMPIRICAL (section 4) |
| What does need a DSP answer? | The **voice free** transition: a second note on a held voice waits on the idle bit, which only the DSP sets again (release state `0x1C083C`, a sample position test inside the SY). With the message-driven mask the ARM spins (about 40,000 status reads in 500 ticks). Needs a live SY chip. | EMPIRICAL |

## 2. Inventory of DSP to ARM reads (evidence)

| # | Signal | DSP address | ARM reader | Evidence |
|---|---|---|---|---|
| 1 | SY voice idle mask (64 bit, 56 voices) | SY `DM(0x92196)`, `0x92197` | wait `0xC01E0694` / `0xC01E06A0` in the voice update; request `0xC01E0728` = `0xC0066E64(SY comm, dev, [sb+4] = 0x92196, 2, 0xC05B0454)`; test `[0xC05B0454 + 4 [r5+4]] & [r5+8]` | run: `[sb] = 6`, `[sb+4] = 0x92196`, status word 0, mask 1 for voice 0 of key 60. SY listing: idle state `0x1C07B0` ORs the voice bit into `DM(0x92196)` (`sy-smp-audio.md` s3). Oracle `sharc-ref` dump: `0xFFFFFFFE 0x00FFFFFF` after the prepare state of voice 0 (VERIFIED) |
| 2 | Release mask | SY `DM(0x9219A)` | not found (the release state `0x1C083C` sets it) | the reader is not decoded: OPEN |
| 3 | Note-on table refill (3 peeks, 4 words) | PNO core 2, ORG core 1, ORG core 2: `DM(0xC1DDC..F)` | `0xC01B1840` -> `0xC00119A4` at `0xC01B1CB4`, `0xC01B1CD0`, `0xC01B1CEC`; table `0xC0549598` stride 16 `{index, address, 0x50, 0x15000000}`; flags `0xC027A908`, chip `0xC027A8E8` | run: `(chip, address) = (1, 0x28B07770), (2, 0x28307770), (2, 0x28B07770)` once per note-on. Result table `0xC05A4AB0 / AC0 / AD0`, `abs(signed byte)` of word 0 gates `0xC01DEAFC` (<= 1) (D143) |
| 4 | Wait with timeout | per `[sb+4]`, 2 words | `0xC01D6840..0xC01D697C` (`0xC00117E8` at `0xC01D6970`, `0xC0066E64` at `0xC01D692C`), timeout `0x258` ticks of the `0x80001DC0` timer | listing; not reached in the runs |
| 5 | Piano voice allocation | SMP status word, address `[object + 0x240]` | `0xC00B64D8..0xC00B659C`, `0xC0066E64` at `0xC00B6588`, repeats until the word is non-zero | listing; `arm_piano_load.STATUS_POLL_PC` |
| 6 | GPIO lines (DSP driven) | pins, no DM word | flow control toggle `0xC006CA48`; SPI ready wait `0xC0011B2C`, `0xC0011C90` (GPIO pair 2 IN `0x01E26070`); DSP boot `0xC024B0F0` | run: 12 + 513 + 6 reads; the harness echoes SET/CLR into IN (`arm_boot.install_gpio_echo`) |
| 7 | Not DSP | EMIFA CS2 `0x60000010/20`, `0x62004005`, GPIO `0x01E26022` | ISRs at `0x800013F8`, `0x80001500` (panel and keybed scan) | run (255 + 128 reads) |

The harness before this work: `0xC0066E64` was a boot stub returning 0 (`arm_bulk_tables`), SPI1 plain RAM served the peeks (the D143 test pattern), `DspAck` set bit 0 by hook before the wait.

Routine dispatch (VERIFIED, `0xC01E06A4..0xC01E0728` and `0xC01B1840`): per device index `i`, `0xC027A908[i]` is 1 for `i = 0..5` (SPI read `0xC00119A4` / `0xC00117E8`, chips FX, PNO, ORG) and 0 for `i = 6..9` (SY, SMP, PNO: a comm message first, then `0xC0066E64`); `0xC027AA20[i]` / `0xC027A8E8[i]` give the chip. Unexercised: 52 of the 62 `bl 0xC0066E64` sites (their address and count come from registers).

## 3. Implementation (opt-in)

`tools/arm_dsp_return.py` (`ReturnSource`): answers a read `(chip, address, count)` from, in order, the SY voice masks (driven by the SY state messages `(0x92414 + 0x38 k, state)`: idle and reset set bit `k`, prepare and go clear it; initial all 56 idle, ASSUMED from the 56 idle literals of the ARM record initialiser `0xC04FECFC`, the OR in the idle state VERIFIED on the oracle), the live chip (`Link.dump`, `D` command), the shadow of every word the ARM sent, the flat initial DSP image, 0. `install` swaps the three routines (the harness stub of `0xC0066E64` is replaced and restored). `arm_dsp_live.py --return-path` wires it into the live link: every grouped message feeds the shadow, the live chip answers its own `(chip, core)`. `tools/arm_dsp_return_run.py` runs one program from one saved state in three modes (legacy, neutral, return).

Tests (`tools/tests/test_arm_dsp_return.py` 12, `test_arm_dsp_return_run.py` 4, 2 added to `test_arm_dsp_live.py`): address alias to `(core, word)`, comm object to chip, mask update by state message (the oracle values `0xFFFFFFFE`, `0x00FFFFFF`), layer order, the three stubs on a fake emulator (a prepared voice reads bit 0 clear, an idle one set), summary helpers. Without the module the tests do not import.

Limits: the live link (D144 Router/Chips) registers every live chip as a layer, `(comm, core)`, so PNO core 2 and ORG chips answer the peeks when they are given with `--chip`; the tables in section 4 were run without live chips (images and shadow only), the live run is in section 4b. SY is not a live chip (no `sharc-live` SY preset), so the mask is message driven, not DSP driven. The `0xC0066E64` stub decodes only the voice update site: an unknown comm object, an address no layer knows, or a count over 4 falls back to the old stub (return 0, no write; counted in `stub_fallbacks`).

## 4. End to end (key 60, 64, 67 at ticks 0, 20, 40, off at tick 100, 500 ticks, from one saved state)

| Program | Mode | SY messages | prepare | voice updates | key-down delay (ticks) | SPI word read | status reads |
|---|---|---:|---:|---:|---|---|---:|
| Acoustic Guitar (SAMPLE) | legacy | 17 | 1 | 1 | 1, none, none | 1 | 0 |
| | neutral | 17 | 1 | 1 | 1, none, none | 1 (zeroed) | 0 |
| | **return** | 17 | 1 | 1 | 1, none, none | 0 (DSP) | 40,683 |
| String Quintet (SAMPLE) | legacy | 19 | 1 | 1 | 1, 1, 1 | 3 | 0 |
| | neutral | 17 | 1 | 1 | 1, none, none | zeroed | 0 |
| | **return** | 17 | 1 | 1 | 1, none, none | 0 (DSP) | 40,683 |
| Grand Pad, 4 Osc Funk Bass (no sample layer) | all three | 0 SY (PNO 12; ORG 12) | 0 | 0 | none | none | 0 |

Reading: (a) `legacy` String Quintet shows the D143 artefact (test pattern read 3, voices vary); `neutral` and `return` agree. (b) The return path reproduces the neutral result: the images and the shadow are 0 at the peeked words, so the hardware peek is consistent with D143's neutral read (EMPIRICAL; the live PNO core 2 / ORG answers, which could differ after DSP passes, are not measured). (c) Key-down delay 1 tick for the first note in all modes: the real mask read does not slow the first voice. (d) Notes 2 and 3 produce no SY message in `neutral` and `return` (also in `legacy` for the guitar): the ARM stays in the voice update wait, which in `return` is the 40,683 status reads, because bit 0 stays clear after voice 0 was prepared and the model has no DSP release. Voice free timing after the note-off is therefore **not measurable** without a live SY (no `frees`). (e) The two programs without a sample layer read nothing from the DSP in a note (synth-only note path: no SPI reads, no SY message).

### 4b. Live chips answer the peeks (String Quintet, `arm_dsp_live.py --return-path`, PNO core 2 and ORG core 2 live)

Data: `docs/research/data/dsp-to-arm-return-live.json`. Chips `PNO:2=org` and `ORG:2=org` (D144 Router/Chips), 800 load, 100 idle, 1500 key, 300 off ticks. The note-on refill made 3 peek calls (12 words): 8 words answered by the live chips (PNO core 2, ORG core 2), 4 by the zero layer (ORG core 1 is not a live chip here). The live chips received 900 messages each in the load phase and 1 PNO2 message in the key and off phases. 0 busy SY voices at the end (no SY chip; the mask is message driven). The values the live chips returned are not recorded in the JSON, so this run shows the wiring answers from the chips, not what the peek gate does with them: the gate effect is not measured (EMPIRICAL, one program).

## 5. Implications

- The premise "the DSP consumes a sample ring and requests refill, the ARM must refill" does not hold for SY: no ARM read of ring state exists in the note path. The sample data reach the SY by its own DMA (D141); an ARM return channel for ring refill is not needed. What the SAMPLE path needs from the DSP is the voice-ready mask and the note-on peek words.
- The note-on peek is a voice-count style gate fed by DSP words of PNO core 2 / ORG (word `0xC1DDC` is the packed parameter buffer of ORG, `org-dsp-map.md`); on this evidence the hardware value is small, as D143 assumed.
- Voice allocation and stealing (second note on a held voice) needs the DSP's own release transition; the next step is an SY chip on the live link.
- Defaults, goldens and `render_hash` do not change.

## 6. Follow-ups

1. **Done, D153** ([sy-live-chip](sy-live-chip.md)): SY preset in `sharc-live`, masks answered from the chip. The 40,682 reads of section 4 belong to a second wait (return `0xC01E26FC`), not to `0xC01E0694`. Voice free and stealing: not shown.
2. Decode the 52 other `0xC0066E64` sites and the reader of `DM(0x9219A)`.
3. Longer live runs of PNO core 2 / ORG answering the peeks (section 4b is one program).
4. SMP status word of the piano voice allocation (item 5): address `[object + 0x240]`, value not measured.

## Reproduce

```bash
uv run pytest tools/tests/test_arm_dsp_return.py tools/tests/test_arm_dsp_return_run.py tools/tests/test_arm_dsp_live.py
C=target/fw_unpacked
PYTHONPATH=tools uv run --with unicorn --with capstone python tools/arm_dsp_return_run.py $C/bank0_CODE.bin $C/bank0_SRAM.bin \
  "factory_programs/Program/Bank A/String Quintet.ns4p" --eeprom target/oled/eeprom.bin \
  --ns4b ".reference/Nord Stage 4 Factory Restore RevK v1.66.ns4b" --images <dir with PNO/ ORG/ core<N>_sram.bin> --out run.json
```

Oracle check of the SY mask states (VERIFIED reset, prepare, go; idle `0x1C07B0` ASSUMED, not dumped separately):

```bash
target/release/sharc-ref target/oracle/sy.bin tools/oracle_scenarios/sy_voice_mask.scn --out target/oracle/vm
# dump 0x92196: ffffffff 00ffffff / fffffffe 00ffffff / fffffffe 00ffffff / ffffffff 00ffffff
```

Disassembly facts (entry points, argument order, the three call sites): `capstone` over `target/fw_unpacked/bank0_CODE.bin` at `0xC00119A4`, `0xC00117E8`, `0xC0066E64`, `0xC01B1CB4/CD0/CEC`, `0xC01E0694`. The run numbers of section 4 are in `docs/research/data/dsp-to-arm-return.json`.
