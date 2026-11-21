# The SY pitch entry `e`, fifth hunt: the flash bus, the ARM claimed flash service and the offset table (negative result, D176)

- **Checked:** 2026-11-20
- **Sources (local, never committed):** SY1SY2 listing `target/oracle/sy2.lst` and image `target/DSP/dsp_image_3_SY1SY2.bin` (OS v1.66 `bank0_CODE@0xC044667C..`), `bank0_CODE.bin`, the boot port log `target/bulk/boot_log.jsonl` ([arm-bulk-tables](arm-bulk-tables.md), 96,111 messages), `tools/arm_sy_slot_probe.py` on Carina Accordion (key 60), `sharc-ref` scenarios of [sy-pitch-entry-writer](sy-pitch-entry-writer.md) and [dsp-flash-dma-model](dsp-flash-dma-model.md). New: `tools/sy_flash_census.py` (`tools/tests/test_sy_flash_census.py`).
- **Question:** Four hunts ([sy-pitch-entry-writer](sy-pitch-entry-writer.md) s1 to s9, D141, D147) found no writer of the 16-bit entry `e` at `DM(0x9354A + index)`. The open leads were the writer of the per-sample offset table `DM(0xB2000..)`, the device behind AMI banks 2 and 3, the SMP chip, blocks with `ns` not a multiple of 32, the loop-end call `0x1C064B`, interrupt vectors 1 and 12 and the step word `DM(0x9242D)`. Which of them writes `e`?

Tags follow `.agents/rules/provenance-in-code.md`. Addresses and counts only; no firmware word, sample or audio value is quoted.

## 1. Short answer

| Lead | Result | Tag |
|---|---|---|
| Writer of `DM(0xB2000..)` | **None.** The SY image zero fills `0xB2000 .. 0xB6002` (one tag 1 block, 16,387 words, no data block inside), the only SY reader is `i0 = 0xB2000` at `0x1C0C46`, the boot log holds **0** SY messages in `0xB2000 .. 0xB5FFF` (290 SY messages), the sample note-on holds 0 (19 messages), and `sharc-ref --watch 0xB2000:0x4003` reports **0** writes in `sy_isr_epchain.scn` (3,888 TCBs), `sy_isr_refill.scn` and `sy_isr_handlers.scn` (positive control: `--watch 0x93FF7:4` logs the staging writes of the same run) | VERIFIED (`tools/sy_flash_census.py`, listing, runs) |
| Device behind AMI banks 1 to 3 | **A flash with an upper address latch**, reached by the ARM through the DSP (section 2). Bank 1 `0x0400_0000 +` = data and command window, bank 2 `0x0800_0000 +` = address latch (and the flash read DMA of D141 writes it first), bank 3 `0x0C00_0000 +` = read window. The boot sends SY `0x04000000 <- 0xF0`, `0x04000145 <- 0x98` and five bank 2 latch writes; the SMP gets the same pattern at `0x280` | VERIFIED (log, ARM listing); "flash" ASSUMED from the command values `0xF0` (reset) and `0x98` (CFI query) |
| New: an ARM claimed flash service inside the SY audio ISR | `DM(0x93DF6) != 0` sends the ISR to a service loop (`0x1C05C6` to `0x1C0857`, `0x1C0871`): it runs the ARM message list again and then serves two mailboxes: **flash read** (`DM(0xB6000)` address, `DM(0xB6001)` count, `0x1C08DB`) and **checksum** (`DM(0xB6510)` count, `DM(0xB6511)` table, `DM(0xB6512)`, buffer pointer `DM(0xB650F)`, `0x1C09A9`, a 256 entry xor table lookup per byte over the flash data) | VERIFIED (listing); roles read from the code, not run |
| Can that service write the pitch window? | **The read service no, the checksum service yes if the ARM asks.** Read service: destination starts at `0x93FF7` (`r7 = 0x93FF7`, `0x1C0925`) and only grows, the window `0x93500..0x9359F` lies below it. Checksum service (`0x1C09A9` calling `0x1C0A67`): its flash DMA destination is the ARM word `DM(0xB650F)`; **run** (`target/oracle/np/crc.scn` = `sy_isr_handlers.scn` set-up, `epchain`, `DM(0xB650F) = 0x9354A`, `DM(0xB6511/0xB6512)` a one entry table, `run 0x1c09a9`): the chain has 3 TCBs, the third is `ext 0xC000000 -> DM 0x9354A`, count 4, `window_hits = 1`. So an ARM that sets `DM(0xB650F)` into the window and claims the bus makes the DSP DMA flash words into the pitch table: **this is the first ARM parameterised flash to DM path that can fill `e`**. No ARM literal or `movw` / `movt` pair names `0xB650F`, `0xB6000` or `0xB6512` (section 3) and the harness never claims the bus, so it was never driven; a computed ARM address is not excluded | VERIFIED (listing, run, `literal_hits`); that the firmware uses it for `e` is ASSUMED |
| ARM literals of the service | `0x93DF6`, `0x93DF7`, `0xB6001` in the device object builder `0xC024C284` and `0x93DF6`, `0x93DF7`, `0xB6510..0xB6512` in the descriptor words `0xC0444E4C..0xC0444E5C`; none in any ARM instruction that sends a message. The only boot messages to these words are `0x93DF6 <- 1 / 0` (claim / release, pcs `0xC0062EAC`, `0xC004B8B8`) and `0xB6510 <- 0` (pc `0xC02488C4`) | VERIFIED (boot log, scan) |
| SMP chip | **No link to the SY.** The SMP has its own port (comm 4), flash probe messages (`0x08000280`, `0x04000285`) and image; the SMP DM `0xB2000..` is a different structure (voice records of stride 7 in its image, 1,480 ARM messages in the boot into `0xB2000..0xB5FFF`, the SY gets 0). The SMP listing has no operand in `0x93500..0x935A0` ([sy-pitch-entry-writer](sy-pitch-entry-writer.md) s1); no DSP to DSP path exists in either listing | VERIFIED (log, earlier listing scan); no link ASSUMED from the missing path |
| Loop-end call `0x1C064B`, vectors 1 and 12 | Not re-run. The ARM never sends a handler opcode: every one of the 290 SY and 1,540 SMP boot messages has opcode 0 (`census_log`, `opcodes: {0: ...}`), plus the 19 note-on and the 545 messages of D111, so the call at `0x1C064B` and the handler bodies are never dispatched; vectors 1 and 12 stay as in D112 (reset code and an `RTI` stub) | VERIFIED (log); core behaviour at loop end stays open |
| `ns` not a multiple of 32 | **Not run.** The unpacker desync of [sample-stream-audio](sample-stream-audio.md) stays open; no new evidence that a block header carries `e` (D112: headers and residuals only) | open |
| Step word `DM(0x9242D)` | Stays 0 in every driven path, consistent with `e = 0` (the zero fill) and the join `step = ratio(e) * 0.05 / word` (D61, `sy_pitch_join.scn`); no new write found | VERIFIED (D61 join), cause not independent |
| Verdict | **Writer not found; five hunts, no hit, one candidate path.** `--sample-pitch fw` stays opt-in (D22, D65, D89). New paths excluded: the offset table, the AMI flash bus from the ARM side, the read mailbox of the claimed service, the SMP chip, the flash boot probe; **not excluded**: the checksum service with `DM(0xB650F)` pointed at the window | negative for what ran; candidate open |

## 2. The flash bus (new)

| Item | Evidence | Tag |
|---|---|---|
| ARM routines that write the SY / SMP external bus | `0xC004AD04` and `0xC004AE74` (slots of the vtable at `0xC029EF7C`, 9 methods `0xC004A8B8 .. 0xC004B850`) write a 16 bit value to a byte address `A` of the DSP external space as **two port messages**: first (only when the upper address changed) the latch `bank 2: 0x08000000 + (A & mask) <- 0x8000 \| ((A >> 24) & 0x7F)`, then the data `bank 1: 0x04000000 + (A & 0xFFFFFF) <- value` (byte pair replicated when the bus width words `[obj + 0xA80]`, `[obj + 0xA84]` differ). Method `0xC004B850` is the release: it sends `DM(0x93DF6) <- 0` (pc `0xC004B8B8`) | VERIFIED (ARM listing, boot log pcs `0xC004ADC0`, `0xC004AE28`, `0xC004AEF8`, `0xC004AF60`, `0xC004B6F4`) |
| What the boot writes | SY: `0x04000000 <- 0xF0`, `0x04000145 <- 0x98`, `0x08000000 <- 0x8000`, `0x08000140 <- 0x8000`, `0x08000040 <- 0` three times. SMP: the same at `0x280` / `0x285` / `0x080` (its own flash) | VERIFIED (`census_log`: SY spaces `dm 283, bank1 2, bank2 5`; SMP `dm 1526, bank1 4, bank2 10`) |
| DSP side of the same bus | The SY listing uses the three bases only in the DMA builders: `0x1C08E9`, `0x1C08F1` (read service), `0x1C0A6C`, `0x1C0A74` (checksum read), `0x1C0C62`, `0x1C0C6B` (audio chain, D141) and `r0 = 0x4000000` at `0x1C0304` (opcode 6 store); `r1 = 0x4000000` at `0x1C079B` is the field spec bit of the voice reset (section 2 of [sy-pitch-entry-writer](sy-pitch-entry-writer.md)), not a bus address. No SY instruction reads or writes the bus for any other purpose | VERIFIED (`grep` of `sy2.lst`) |
| Reading | Banks 1 to 3 are one flash seen through three windows; the audio chains of D141 (A latch write, X, B read of 8 words) are the sample reads, the service loop is how the ARM reads and checksums the same flash through the DSP. The sample data and any per-sample directory therefore live in the flash, and what the ARM learns from it reaches the SY only as port messages (opcode 0 stores), which the harness captures: none address `0x93500..0x935A0` or `0xB2000..0xB5FFF` | VERIFIED (messages), reading ASSUMED |

The harness cannot run this path: the SPI master transfer and the SPI block download are stubs ([arm-bulk-tables](arm-bulk-tables.md) s2), so the ARM device object of `0xC024C284` and the flash file system behind it never answer. The stub `0xC0240548` was read here: it sends one value to the DSP address `[descriptor + 0xC]` (the descriptor `0xC0444E68` holds `0x92518` there, `0x92519`, `0x940A0`, `0x940A1`, `0x9409F` in its other slots) and polls for the answer, i.e. a boot kernel mailbox handshake, not a table (VERIFIED, listing; the object to descriptor link is ASSUMED from [arm-bulk-tables](arm-bulk-tables.md) s2).

## 3. Reproduce

```bash
python tools/sy_flash_census.py target/fw_unpacked/bank0_CODE.bin \
    target/DSP/dsp_image_3_SY1SY2.bin target/bulk/boot_log.jsonl
uv run pytest tools/tests/test_sy_flash_census.py
cargo build --release -p nord-sharc --bin sharc-ref
for s in sy_isr_epchain sy_isr_refill sy_isr_handlers; do
  target/release/sharc-ref target/oracle/sy.bin tools/oracle_scenarios/$s.scn --out target/oracle/np/$s --watch 0xB2000:0x4003
done
grep -n -i -E '93df[0-7]|0xb6[0-9a-f]{3}' target/oracle/sy2.lst   # service loop and mailboxes
```

Boot loop counts: `tools/arm_boot_chip_objects.py` (tests in `tools/tests/test_arm_boot_chip_objects.py`).

Census output of this run (VERIFIED): offset table zero fill covers = True, data block = False; pitch window zero fill covers = False (the `0x93580` word block), data block = True; SY 290 messages, opcodes `{0: 290}`, pitch hits 0, offset hits 0; SMP 1,540 messages, pitch hits 0, offset hits 1,480; ARM literals only at `0xC024C370..0xC024C384` and `0xC0444E4C..0xC0444E5C`. `sy_isr_stream.scn` needs `target/oracle/weltmeister_s0.stream` (not present in this checkout), so the stream run of D112 was not repeated.

## 4. The ARM boot function `0xC0247B64` and its message loops (excluded, ASSUMED reading)

The two ARM calls of the stubbed block download `0xC0240548` (`0xC0248888`, `0xC02495D4`) sit in the OS boot function `0xC0247B64` (strings: "Sample Flash Error", "FFS Flash", "Ver Flash", "Production Mode", "SampLib"). It builds one chip object per DSP (ctor `0xC0240744`: `[+0] = chip` 3 = SY, 4 = SMP; `[+4]` = the descriptor `0xC0444E4C` / `0xC0444E68`, `[+8]`, `[+0xC]`, `[+0x10]` = 0) and, per chip, runs two message loops (`0xC0248630..0xC0248740` over 7 byte records, `0xC0248760..0xC0248860` over 9 byte records: a 16 bit value at record bytes 7 and 8, a 32 bit value at bytes 3 to 6, address pair advancing by 2).

| Check | Result | Tag |
|---|---|---|
| Are the loops run in the harness boot? | Hooks at `0xC0248744` and `0xC024860C` (unicorn, `arm_boot_full`): both chips reach the loops with **count 0 for the 7 byte and the 9 byte list**, so the boot sends no record from them; the only message after the block download is `DM(0xB6510) <- 0` (pc `0xC02488C4`) | VERIFIED (run) |
| Do they target the pitch window? | Their start words are the chip object fields `[+8]`, `[+0xC]`, written only by the ctor (value 0, same fields the flash write routines of section 2 use as the address latch cache); no ARM store of a window address is visible. Read as the flash programming records of the "FFS / Ver Flash" path, **not** a table loader. A record source (flash directory) could make them non-empty on hardware | reading ASSUMED; not excluded by a run with a flash |
| Descriptor | `0xC0444E4C` (SY): mailbox `0xB6512`, `0xB6511`, `0xB6510`, flags `0x93DF6`, `0x93DF7`, sizes `0x4000`, `0x400`; `0xC0444E68` (SMP): `0x940A1`, `0x940A0`, `0x9409F`, `0x92518`, `0x92519`, `0x4000`, `0x400`. `0x4000` words equals the zero filled buffer `0xB2000..0xB5FFF` | VERIFIED (dump) |

So the 16 K word region `0xB2000..0xB5FFF` is the block buffer of this protocol (size `0x4000`, chunk `0x400`), which the harness stubs out (SPI block download, SPI master transfer). Whether the audio ISR's `DM(0xB2000 + m3)` read is that same buffer or a sample directory is not decided.

## Implications

- The SY never receives, from the ARM or from its own DMA, any word for `0x9354A..0x9358D` or `0xB2000..0xB5FFF` in the paths a harness can run; both regions are zero in the image. The remaining explanation is **hardware state the harness does not have**: the flash file system and its device object (`0xC024C284`), which feed the ARM with sample directory data the stand-in sample bypasses.
- The next productive step is on the ARM side, not the DSP: run the ARM flash driver against a flash model that answers the SPI reads of `DM(0xB6000..)` (read service) and see which sample load or voice start path then sends a message into the two windows. That needs a flash image, which stays local.

### 4.1 The `DM(0xB6510)` command word and the "Ver Flash" reading (2026-11-20)

The ARM routine after the block download call (`0xC0248888 .. 0xC0248A50`, same function) does, per chip: `0xC0240548(obj, 1)` (writes `1` to `[descriptor + 0xC]` = `DM(0x93DF6)`, the bus claim), then sends `[obj + 0x10]` to `[descriptor + 8]` = `DM(0xB6510)` (the boot log message `0xB6510 <- 0`, pc `0xC02488C4`) and **polls `DM(0xB6510)` through the SPI read `0xC0066E64` until it reads 0** (loop `0xC02488F8`). The DSP side clears it at `0x1C09BF` (`dm(0xb6510) = m0`) when the checksum service has run. So `DM(0xB6510)` is the command word of that service, claimed by `0x93DF6`, and the function carries the string "Ver Flash". Reading (ASSUMED): the checksum service **verifies the sample flash** (xor table lookup per flash byte, result stored through `DM(0xB6511)`), it is not a table loader; this lowers the weight of lead 1 in the list below (the destination `DM(0xB650F)` is a verify buffer). The reading is not run: the harness stubs both the claim and the poll.

### 4.2 Flash model run (2026-11-20)

`tools/arm_flash_model.py` (command state, bank 2 latch, CFI / autoselect / array words, all device values ASSUMED) and `tools/arm_flash_boot.py` (answers the SY / SMP reads of `0xC0066E64` in the `0x04000000` window, feeding the chip's port messages to the model first; the boot's real EMIFA stores are logged with `arm_bulk_tables.real_port_capture`). Result of the harness boot, per chip:

| Step | SY | SMP |
|---|---|---|
| Commands before the first read | `0xF0`, `0x90` (27 command or latch messages in all) | same (58 in all) |
| Autoselect reads | offsets `0`, `1`, `0xE`, `0xF` | same |
| After `0x98`: CFI reads | five words `0x4000040..44` | five words `0x4000080..84` |
| Array reads, programs, erases | none | none |
| Boot result | reaches main loop `0xC00A0248` (433 M instructions), same as with the stub that returns 0 | same |

So the **boot** only identifies the flash (autoselect plus CFI size / command set) and never reads sample data; the sample directory and the pitch entry source are read later, by the file system on a program or sample load, which no harness path drives yet. The model is ready for that step: drive a sample load (`arm_piano_load` style) with the model attached and a flash image behind `FlashChip.array`. No flash image is available locally (the backup holds files only), so the content stays the blocker. Status: the model is VERIFIED to run in the boot (`python tools/arm_flash_boot.py ...`); its device values are ASSUMED.

## Leads for the DSP-image hunt in NS4 firmware

Addresses and routines to locate (all in `bank0_CODE` unless stated), with the reason:

| # | Locate | Why |
|---|---|---|
| 1 (lower weight, section 4.1) | Every ARM store that writes `DM(0xB650F)` (checksum service buffer pointer): `movw` / `movt` forms are absent, so look for a computed address, a store at descriptor `0xC0444E4C` plus an offset, and users of the descriptor object at `0xC0444D28` | Run shows the service's flash DMA can target `0x9354A` when this word points there (`sy_flash_service_crc.scn`); it is the only path found that can fill `e` |
| 2 | The ARM flash device object builder `0xC024C284` (calls `0xC0068BA4`, device at `[obj + 0xAC]`) and its read API: who calls it and with which SY addresses | It carries the `0xB6000` read mailbox; the sample directory the stand-in bypasses would come through it |
| 3 | Writers of `DM(0xB6000)` and `DM(0xB6001)` (flash read address and count): no ARM literal exists, so find the SPI write routine variants (`0xC0066E64` callers outside the 10 exercised, 52 sites) | The read service stages flash data at `DM(0x93FF7 ..)`; the ARM may copy its content into the pitch table by messages |
| 4 | The block download body behind `0xC0240548` (second call site `0xC02495D4`) and the SPI DMA opcode 9 sender | The `0x4000` word descriptor size equals the `0xB2000..0xB5FFF` buffer; a block download into it is unmodelled |
| 5 | The DSP image side: any `DM(0xB2000 + k)` reader other than `0x1C0C46` in the SY and SMP images, and the SMP routine that fills `0xB2073 + 7 k` | Decides whether `0xB2000..` is a directory or a staging buffer |
| 6 | `ns` not a multiple of 32 blocks (unpacker handler `0x1C1C44`) and the loop-end call at `0x1C064B` | Open since D112; unrelated to `e` unless a header carries it |

## Follow-ups

1. Model the flash behind the ARM device object: answer the SPI reads that the service loop would produce (`0xB6000` address, `0xB6001` count into `0x93FF7 ..`) in `tools/arm_dsp_return.py`, then re-run the sample load and watch the two windows.
2. (Done in section 1, row 'Can that service write': the DMA reaches the window when `DM(0xB650F)` points there.) Next: find the ARM code that would set `DM(0xB650F)` (computed address, `0xC03B9238` descriptor users, the flash file system) and drive it with a flash image.
3. `ns` not a multiple of 32 and the loop-end call stay open (section 1); neither depends on this hunt.

## Update 2026-11-21 (D196)

The candidate in section 1 (row 'Can that service write') is closed by [sy-flash-service-hunt](sy-flash-service-hunt.md): the image seeds `DM(0xB650F) = 0x93FF7`, so the service writes the staging buffer, not the window; the `0x9354A` poke of this note's run is not firmware behaviour.
