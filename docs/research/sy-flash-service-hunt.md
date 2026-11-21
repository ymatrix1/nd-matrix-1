# The SY pitch entry `e`, sixth hunt: the ARM claimed flash service has a fixed destination (negative result, D196)

- **Checked:** 2026-11-21
- **Sources (local, never committed):** SY1SY2 raw image `target/DSP/dsp_image_3_SY1SY2.bin`, listing `target/oracle/sy2.lst` and flat image `target/oracle/sy.bin`, ARM code `target/fw_unpacked/bank0_CODE.bin` (OS v1.66), `sharc-ref` with the `epchain` model ([dsp-flash-dma-model](dsp-flash-dma-model.md)), prior notes [sy-flash-bus-hunt](sy-flash-bus-hunt.md) (D176) and [sy-pitch-entry-writer](sy-pitch-entry-writer.md). New: `tools/sy_flash_service_hunt.py` (`tools/tests/test_sy_flash_service_hunt.py`), scenario `tools/oracle_scenarios/sy_flash_service_default.scn`.
- **Question:** D176 found one path that can fill `e`: the checksum service `0x1C09A9` DMAs flash words to the DM address held in `DM(0xB650F)`, so an ARM that points that word at the pitch window and claims the bus fills `DM(0x9354A..)`. Does the ARM (or the image) ever set `DM(0xB650F)` to the window, and what does the service write with the values the firmware really uses?

Tags follow `.agents/rules/provenance-in-code.md`. Addresses and counts only; no firmware word, sample or audio value is quoted.

## 1. Short answer

| Question | Result | Tag |
|---|---|---|
| What is `DM(0xB650F)` before the ARM touches it? | **`0x93FF7`, the staging buffer.** The SY image carries a 260 word data block at `0xB650F` (header at image offset 29300): `DM(0xB650F) = 0x93FF7`, `DM(0xB6510) = 0` (count), `DM(0xB6511) = 0x941F7` (table pointer), `DM(0xB6512) = 0x96000` (list pointer), then the 256 entry CRC-32 table at `0xB6513`. The D176 run poked the word to `0x9354A`; nothing in the firmware does | VERIFIED (`python tools/sy_flash_service_hunt.py ...`, test `test_image_defaults_reads_the_data_block_and_zero_fill`) |
| Run of the service with the image default | `sharc-ref` on `sy_flash_service_default.scn` (D176 `crc.scn` set-up, no poke of `0xB650F`): 1 chain, 3 TCBs, the third `ext 0xC000000 -> DM 0x93FF7`, count 4, **`window_hits = 0`**, no write to `0x93500..0x9359F` (`--watch`) | VERIFIED (run) |
| Does the DSP ever store to the service words? | Direct stores in the SY listing: `DM(0x93DF7)` (`0x1C085E`, `0x1C08D7`, the busy word), `DM(0xB6001)` and `DM(0xB6510)` cleared as completion flags (`0x1C08E1`, `0x1C09BF`). **None to `DM(0xB650F)`**, `0xB6000`, `0xB6511` or `0xB6512`; `0xB650F` is read at `0x1C0A12` and `0x1C0AB6` only | VERIFIED (listing scan `dsp_writers`; indexed stores are not visible to a text scan) |
| Does the ARM name `DM(0xB650F)`? | **No.** A linear constant tracker over the whole ARM code (`arm_const_hits`: literal loads, `mov` / `mvn` / `movw` / `movt`, register copies, `add` / `sub` / `orr` immediates, reset at branches) finds the service words only at `0xC024C2E8` (`0xB6001`) and `0xC024C304` (`0xB6000`), both in the device object builder `0xC024C284`. The descriptor `0xC0444E4C` lists `0xB6510..0xB6512`, `0x93DF6`, `0x93DF7` and has **no slot for `0xB650F`**, so the ARM cannot reach it through the descriptor either, and no `descriptor value - 1` construction exists | VERIFIED (tool run, 2 hits, none for `0xB650F`); computed addresses across basic blocks ASSUMED absent |
| Does any ARM constant name the pitch window? | **No.** The tracker over all 160 window addresses `0x93500..0x9359F` reports 0 hits in the ARM code | VERIFIED (tool run) |
| What is the device object `0xC024C284`? | Not a table loader. It fills a configuration record for the driver constructor `0xC0068BA4`: bank base `0x08000000`, read address word `DM(0xB6000)`, count word `DM(0xB6001)`, **staging buffer `0x93FF7`**, claim words `0x93DF6` / `0x93DF7`, chunk `0x800` words, and stores the device at `[obj + 0xAC]`. That is the ARM end of the **read service**, whose destination `r7 = 0x93FF7` only grows (`0x1C0925`, D176). Both services therefore land flash data in `0x93FF7..` and the ARM reads it back through SPI; neither is parameterised to the window | VERIFIED (ARM disassembly `tools/arm_xref.py dis ... 0xC024C284 0xC024C370`, listing) |
| Verdict | **Writer not found; six hunts, no hit. The ARM claimed flash service is closed as a path for `e`:** its buffer is the staging area `0x93FF7` by image default, no ARM or DSP code changes it, and the window is not named anywhere in the ARM code. `--sample-pitch fw` stays opt-in (D22, D65, D89) | negative for what ran and what the code can name |

## 2. What was not built, and why

The brief asked for a flash image and SPI read answers for the ARM device object. They were not built, for a reason found in section 1: the service destinations do not depend on flash *content* (`DM(0xB650F)` and the read staging address are fixed by the image and the object builder), so a flash image can only change the bytes that land at `0x93FF7`, never the address range. `tools/arm_flash_model.py` / `arm_flash_boot.py` (D176 section 4.2) already answer the boot-time identification reads; the sample directory content remains unavailable locally (the restore backup holds files, not the raw flash). No firmware routine outside the files read here was needed, so the DSP-image hunt session was not asked.

## 3. Reproduce

```bash
python tools/sy_flash_service_hunt.py target/fw_unpacked/bank0_CODE.bin \
    target/DSP/dsp_image_3_SY1SY2.bin target/oracle/sy2.lst
uv run pytest tools/tests/test_sy_flash_service_hunt.py
cargo build --release -p nord-sharc --bin sharc-ref
python -c "open('target/oracle/np/ext.bin','wb').write(bytes([0xA5])*0x400000)"
target/release/sharc-ref target/oracle/sy.bin tools/oracle_scenarios/sy_flash_service_default.scn \
    --out target/oracle/np/crcdef --watch 0xB650F:1 --watch 0x93500:0xA0
uv run --with capstone python tools/arm_xref.py dis target/fw_unpacked/bank0_CODE.bin 0xC024C284 0xC024C370
```

Output of this run: `image DM(0xB650F) = 0x93FF7 in pitch window: False`, `0xB6510 = 0`, `0xB6511 = 0x941F7`, `0xB6512 = 0x96000`; DSP direct stores to service words 4 (none to `0xB650F`); ARM constant tracker hits 2 (`0xC024C2E8`, `0xC024C304`); ARM constants naming a pitch window address 0; epchain `window_hits=0`, third TCB `ext 0xC000000 -> DM 0x93FF7 count 4`.

## Implications

- Every DSP path that moves flash data (audio chains D141, read service, checksum service) writes outside `0x93500..0x9359F` with the firmware's own parameters; the only way found to hit the window was to poke the checksum buffer pointer, which is not firmware behaviour. The D176 "first path that can fill `e`" is a harness construct.
- What stays: `e` may be written by an ARM **message with a runtime-computed address** (`0x9354A + voice`) after the ARM parsed a sample directory read through the claimed read service. The tracker sees no constant, but a base from a data table plus an index is invisible to it. Every ARM to SY message captured so far (290 boot, 19 note-on, 119 programs, D124) has no address in the window, but all ran with the stand-in sample, not a real directory.

## Follow-ups

1. Capture the ARM to SY messages of a **real sample load** (real directory read through the read service of `0xC024C284`, `tools/arm_flash_model.py` answering `DM(0xB6000/1)` reads with the file system of the restore image): the one remaining place `e` can come from. Needs the sample file system layout (OQ in [sample-stream-audio](sample-stream-audio.md)).
2. Trace the ARM message send routine's address argument dynamically (a watch on every send whose DSP address lies in `0x93500..0x935A0`, across all 119 programs with the neutral SPI1 harness of D143) to close the computed-address case.
3. `ns` not a multiple of 32 blocks and the loop-end call `0x1C064B` stay open (D176).
