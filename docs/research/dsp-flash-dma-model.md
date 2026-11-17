# SY sample path: the external-port DMA chain model, no SPI flash on the SY, and what stays open (D141)

- **Checked:** 2026-11-17
- **Sources (local, never committed):** SY1SY2 listing `target/oracle/sy2.lst` and the flat image `target/oracle/sy.bin` (OS v1.66 `bank0_CODE@0x4FD980`, [sy-dsp-map](sy-dsp-map.md)), the ARM init script record at `bank0_CODE@0xC0503600..0xC05051E8`, the ADSP-214xx Hardware Reference rev 1.1 (`.reference/sharc/ADSP-214xx_hwr_rev1.1.pdf`, text dump under `target/tmp/`), prior notes [sy-pitch-entry-writer](sy-pitch-entry-writer.md) (D111, D112), [arm-dsp-capture](arm-dsp-capture.md) (D124), [sharc-sy1sy2-smp](sharc-sy1sy2-smp.md). New: `crates/nord-sharc/src/core/periph/epchain.rs`, `sharc-ref` command `epchain`, `tools/ns4_arm_init_script.py --tcb / --tcb-summary`, scenario `tools/oracle_scenarios/sy_isr_epchain.scn`. The task brief (an earlier session) and the `sample-voice-gate` notes were read as data; that session had published nothing when this was written.
- **Question:** Is there an SPI flash plus DMA path in the SY / SMP images that produces the pitch entry `e` and the sample stream by itself, and do the SAMPLE programs then render?

Tags follow `.agents/rules/provenance-in-code.md`. No firmware word, sample or audio value is quoted; addresses, counts and program counts only.

## 1. Short answer

| Question | Result | Tag |
|---|---|---|
| Is there an SPI flash on the SY or SMP path? | **No SPI flash behind the SY SPI block** (this does not exclude a parallel flash on AMI banks 2 / 3). The SY SPI block (`SPICTL 0x1000`, `IISPI..SPIDMAC 0x1080..0x1084`) is the **ARM command link**: opcode 9 (`0x1C03AA..0x1C03DC`) programs a receive DMA (`II` = bits 19..0 of the message word, `C` = bits 31..20, `IM = 1`, chain pointer `m0`); the reset code `0x1C108B` enables it and unmasks SPIHI. Nothing reads a flash through it, and the ARM never sends opcode 9 ([sy-pitch-entry-writer](sy-pitch-entry-writer.md) s4) | VERIFIED (listing) |
| Which path moves the sample data? | The **external port**: DMA channel 0 (`DMAC0 0x180B`, `CPEP0 0x1826`, `EIEP0..ICEP0 0x1820..0x1825`) with chain loading, started by the audio ISR code at `0x1C0900..0x1C0B59` and `0x1C0BF9..0x1C0CA7`. The start-up code `0x1C0CA8` sets `EPCTL` and `AMICTL1..3` (`0x1805..0x1807` = `0x138B`, `0x084B`, `0x00D3`), so the DSP reaches **asynchronous memory banks** 2, 3 and 4 (`0x0800_0000`, `0x0C00_0000`, `0x1000_0000`) | VERIFIED (listing, HRM register table) |
| What do the chains look like? | Per slot **three TCBs**: A = DM to external, 1 word (`ext 0x0800_0000 + 2 * off`, source `DM(0x935D4..0x935E5)`), X = external to DM, 1 word (`ext 0x935D2` to `DM(0x935D3)`, a read after the write, ASSUMED to order it), B = external to DM, **8 words** (`ext 0x0C00_0000 + off`, destination the 128-word staging buffer of the voice, `DM(0x93FF7 + 8 k)`). The shape is the same in the double buffered chain (`0x1139E5`, `0x1135F5`) and in the 68-slot static chain of the ARM init script (`0x136008`) | VERIFIED (listing; run, section 3) |
| Where does the offset `off` come from? | `0x1C0C2D..0x1C0C70`: `r2 = DM(0xB2000 + m3) + r1 + 0x20` (a per-sample offset table at `DM(0xB2000..)` indexed through the voice record), `off = r2 & 0x7FFFFF` (23 bits), the upper bits are stored in the voice record (`DM(i3 + 4)`), the bank 2 address is `0x0800_0000 + 2 * off`, the bank 3 address `0x0C00_0000 + off` (data) | VERIFIED (listing); reading of "latch write, then data read" is ASSUMED |
| Does any TCB write the pitch window `DM(0x93500..0x935A0)`? | **No.** All 204 TCBs of the static table: destinations `0xB64CB..0xB650E` (A source), `0x935D3` (X), `0x93FF7..0x9442D` (B); the run: 3,888 TCBs in 81 chains, destinations `0x935D3` and `0x93FF7..0x9406F`, **0 window hits** | VERIFIED (`--tcb-summary`, `sy_isr_epchain.scn`) |
| Do SAMPLE programs now render? | **Not measurable, 0 of 119 changed.** The model is a standalone oracle module; no ARM harness run feeds the SY chip, there is no external memory image (flash or SDRAM) and the 59 programs that stopped at the **ARM** voice gate never sent a word to the DSP; D143 ([sample-voice-gate](sample-voice-gate.md)) later traced that to the SPI1 stand-in RAM of the harness and neutralised it (110 of 119 reach the voice update), so the render question needs a re-run on the neutral harness | VERIFIED (D124 numbers), the rest not run |
| Verdict | **Negative, narrowly scoped.** Not found in the TCBs the audio ISR builds in `sy_isr_epchain.scn` (voice offsets 0, no external image) nor in the 204 static TCBs; not an SY SPI read and not an ARM message. Chains built for non-zero voice offsets were never driven (follow-up 3), so this does not exclude the external-port path in general | |

## 2. Registers and the TCB format (HRM)

| Item | Value | Tag |
|---|---|---|
| EP channel 0 registers | `DMAC0 0x180B`, `EIEP0 0x1820`, `EMEP0 0x1821`, `ECEP0 0x1822`, `IIEP0 0x1823`, `IMEP0 0x1824`, `ICEP0 0x1825`, `CPEP0 0x1826` | VERIFIED (HRM table) |
| TCB storage order (ascending) | `CP, EM, EI, C, IM, II`; a chain pointer is the address of `II` (bits 18:0), bit 19 = PCI, bit 20 = CPDR | VERIFIED (HRM listing 4-7, table 3-13) |
| CPDR | of the pointer that loads a TCB: 0 = write internal memory (external read), 1 = internal read (external write), with `DMAC0.OFCEN` set; the ISR writes `DMAC0` bits `0x125` after `CPEP0` | VERIFIED (HRM text) |
| TCB address alias | IOP pointer `0x36008` is the normal word `0xB6008` | EMPIRICAL (ISR writes the fields at `0xB6005..`; same for all 204 static TCBs) |
| Static table | one type 4 record `[4][1224][0xB6003]` at `0xC05038BC`: 68 slots x 18 words, TCBs at `+5`, `+11`, `+17` of each slot; A and X static, B count written by the ISR | VERIFIED (`ns4_arm_init_script.py --tcb-summary`, `tools/tests/test_ns4_arm_init_script.py`) |

## 3. The model and the run

`core::periph::epchain` (`Core::enable_epchain`, scenario command `epchain [path=FILE base=ADDR]`, off by default): a write of `DMAC0` with `DEN` set and a non-zero `CPEP0` walks the chain at once (completion is ASSUMED instant, no status bits, no interrupt, at most 1,024 TCBs, count at most 4,096), reads and writes a sparse external word store, logs every TCB and counts transfers into `DM(0x93500..0x935A0)`. A read of an external word nothing provides leaves the destination unchanged (ASSUMED; keeps the data of the `refill` / `stream` models). Unit tests: chain of two TCBs (latch write then block read), window counter, loop and oversize abort, register claim, `Core` wiring.

```bash
cargo build --release -p nord-sharc --bin sharc-ref
python tools/ns4_arm_init_script.py target/fw_unpacked/bank0_CODE.bin --tcb-summary
target/release/sharc-ref target/oracle/sy.bin tools/oracle_scenarios/sy_isr_epchain.scn --out target/oracle/epc --watch 0x93500:0xA0
```

| Measure (`sy_isr_epchain.scn`, 400 audio ISRs after the prepare messages, `refill 0`) | Value |
|---|---|
| chains / TCBs | 81 / 3,888 (16 slots x 3 per chain) |
| words external to DM / DM to external | 11,664 / 1,296 |
| unmapped external reads | 11,664 (no external image given) |
| transfers into the pitch window | **0** (also `--watch 0x93500:0xA0`: no write) |
| shape A | DM to ext, 1 word, ext `0x0800_5900 + 0x10 k`, source `DM(0x935D4..0x935E5)` (`0x935E2` in the first chain) |
| shape X | ext to DM, 1 word, `0x935D2` to `DM(0x935D3)` |
| shape B | ext to DM, 8 words, ext `0x0C00_2C80 + 8 k`, destination `DM(0x93FF7 + 8 k)` |

Static census (`--tcb-summary`): A 68 TCBs `0xB64CB..0xB650E` (a DM source, not a destination); X 68 TCBs `0x935D3`; B 68 TCBs `0x93FF7..0x9442E`, checked with the ISR count of 8 words (`r4 = 8`, listing `0x1C0AF6`; the static table holds 0); window hits 0 in all three.

## 4. What this means for the gate and the 119 programs

| Item | Result |
|---|---|
| Voice gate (59 of 119 stopped, D143: harness artefact, now 110 of 119 voiced) | The stop was **ARM side**: the voice event `0xC01EC690 -> 0xC01E72C4` ends before the voice update ([arm-dsp-capture](arm-dsp-capture.md) s2), before any message reaches the SY. A flash or DMA model on the DSP cannot change it. The `sample-voice-gate` session owns that diagnosis |
| The 60 voiced programs | They reach the SY, which then needs (1) the offset table `DM(0xB2000..)` filled, (2) the external banks 2 and 3 holding the sample data, (3) `e`. Who fills (1), what device sits behind bank 2 / 3 (the flash, or SDRAM that the ARM loader fills; the sample loader `0xC01D710C` sends 0 DSP messages) and who writes (3) are all **not found** |
| Level and timing vs the native sample path | not measured: no SAMPLE program is run through the chip with an external image |

Reproduce the SPI and EP channel readings (the listing is local, made with `tools/adi_dis.py` over the SY image):

```bash
grep -n -E 'dm\(0x(1000|108[0-4])\)' target/oracle/sy2.lst   # SPICTL / IISPI..SPIDMAC users: 0x1C03AA.., 0x1C108B..
grep -n -E 'dm\(0x(180b|182[0-6])\)' target/oracle/sy2.lst   # DMAC0 / EP channel 0 users: 0x1C0900.., 0x1C0AB9.., 0x1C0B36..
```

Model limits (all ASSUMED): `DMAC0` reads return the last written value (`DEN` never clears, a program that polls for completion would spin); `epchain path=` is capped at 64 MiB; `MAX_TCBS`, `MAX_COUNT` and `LOG_CAP` only cut loops and reports.

## Implications

- The "SPI flash" hypothesis of D124 is refuted for the SY: SPI is the ARM link; the sample bytes come through AMI banks 2 and 3 by external-port DMA chains that the audio ISR builds itself (shape A, X, B).
- `e` is not carried by any of the 204 static TCBs nor by 3,888 run TCBs (offsets 0, no external image); with D111 / D112 the SY image has now no store, no handler, no ISR, no stream word and no DMA destination in the window. Remaining candidates: the SMP chip (its own `0x93542..0x93580` records), a transfer the ARM starts on the bank 2 / 3 devices, or a table the SY reads from an external address that is not DM (check whether `DM(0x9354A + index)` is only a cache the SY itself fills from bank 3 by a TCB with a larger index, not yet exercised because the voice offsets stay 0 here).
- `--sample-pitch fw` stays opt-in (D65, D89); no default, golden or `render_hash` moves.

## Follow-ups

1. Find the writer of `DM(0xB2000..)` (the per-sample offset table) in the ARM code: a bulk store the harness did not capture (path 2) or a DSP DMA. Run `arm_dsp_capture` with a memory watch on the SY bank 2 / 3 window of the ARM bus (the ARM must reach the same devices to load samples).
2. Identify the device behind AMI bank 2 / 3 from the board (service manual in `.reference/`) and the ARM's AMI set-up; only then an external image (`epchain path=FILE base=ADDR`) is more than a placeholder.
3. Drive the chain with voice offsets that are not 0 (poke `DM(0xB2000..)` and the record fields) and count whether any TCB moves outside the staging buffer.
4. After `sample-voice-gate` lands: re-count the 119 programs per gate result before judging any render gain.
