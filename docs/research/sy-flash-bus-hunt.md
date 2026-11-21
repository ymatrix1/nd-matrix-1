# The SY pitch entry `e`, fifth hunt: the flash bus, the ARM claimed flash service and the offset table (negative result, D171)

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
| Can that service write the pitch window? | **No.** The read service moves flash words into `DM(0x93FF7 ..)`, the pointer starts at `0x93FF7` (`r7 = 0x93FF7`, `0x1C0925`) and only grows (`r7 = r7 + r4`); the pitch window `0x93500..0x9359F` lies below it. The checksum service reads its buffer, its only stores are the result (`dm(i0, m1) = r2` at `0x1C0A54`, table pointer `DM(0xB6511)`) and its own work words `DM(0xB6613..0xB661C)`; its DMA destination `DM(0xB650F)` is an ARM word, and **no ARM literal or `movw` / `movt` pair names `0xB650F`, `0xB6000` or `0xB6512`** (section 3) | VERIFIED (listing, `literal_hits`); a computed ARM address is not excluded |
| ARM literals of the service | `0x93DF6`, `0x93DF7`, `0xB6001` in the device object builder `0xC024C284` and `0x93DF6`, `0x93DF7`, `0xB6510..0xB6512` in the descriptor words `0xC0444E4C..0xC0444E5C`; none in any ARM instruction that sends a message. The only boot messages to these words are `0x93DF6 <- 1 / 0` (claim / release, pcs `0xC0062EAC`, `0xC004B8B8`) and `0xB6510 <- 0` (pc `0xC02488C4`) | VERIFIED (boot log, scan) |
| SMP chip | **No link to the SY.** The SMP has its own port (comm 4), flash probe messages (`0x08000280`, `0x04000285`) and image; the SMP DM `0xB2000..` is a different structure (voice records of stride 7 in its image, 1,480 ARM messages in the boot into `0xB2000..0xB5FFF`, the SY gets 0). The SMP listing has no operand in `0x93500..0x935A0` ([sy-pitch-entry-writer](sy-pitch-entry-writer.md) s1); no DSP to DSP path exists in either listing | VERIFIED (log, earlier listing scan); no link ASSUMED from the missing path |
| Loop-end call `0x1C064B`, vectors 1 and 12 | Not re-run. The ARM never sends a handler opcode: every one of the 290 SY and 1,540 SMP boot messages has opcode 0 (`census_log`, `opcodes: {0: ...}`), plus the 19 note-on and the 545 messages of D111, so the call at `0x1C064B` and the handler bodies are never dispatched; vectors 1 and 12 stay as in D112 (reset code and an `RTI` stub) | VERIFIED (log); core behaviour at loop end stays open |
| `ns` not a multiple of 32 | **Not run.** The unpacker desync of [sample-stream-audio](sample-stream-audio.md) stays open; no new evidence that a block header carries `e` (D112: headers and residuals only) | open |
| Step word `DM(0x9242D)` | Stays 0 in every driven path, consistent with `e = 0` (the zero fill) and the join `step = ratio(e) * 0.05 / word` (D61, `sy_pitch_join.scn`); no new write found | VERIFIED (D61 join), cause not independent |
| Verdict | **Writer not found; five hunts, no hit.** `--sample-pitch fw` stays opt-in (D22, D65, D89). New paths excluded: the offset table, the AMI flash bus from the ARM side, the claimed flash service, the checksum service destination, the SMP chip, the flash boot probe | negative, scoped to the paths run |

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

Census output of this run (VERIFIED): offset table zero fill covers = True, data block = False; pitch window zero fill covers = False (the `0x93580` word block), data block = True; SY 290 messages, opcodes `{0: 290}`, pitch hits 0, offset hits 0; SMP 1,540 messages, pitch hits 0, offset hits 1,480; ARM literals only at `0xC024C370..0xC024C384` and `0xC0444E4C..0xC0444E5C`. `sy_isr_stream.scn` needs `target/oracle/weltmeister_s0.stream` (not present in this checkout), so the stream run of D112 was not repeated.

## Implications

- The SY never receives, from the ARM or from its own DMA, any word for `0x9354A..0x9358D` or `0xB2000..0xB5FFF` in the paths a harness can run; both regions are zero in the image. The remaining explanation is **hardware state the harness does not have**: the flash file system and its device object (`0xC024C284`), which feed the ARM with sample directory data the stand-in sample bypasses.
- The next productive step is on the ARM side, not the DSP: run the ARM flash driver against a flash model that answers the SPI reads of `DM(0xB6000..)` (read service) and see which sample load or voice start path then sends a message into the two windows. That needs a flash image, which stays local.

## Follow-ups

1. Model the flash behind the ARM device object: answer the SPI reads that the service loop would produce (`0xB6000` address, `0xB6001` count into `0x93FF7 ..`) in `tools/arm_dsp_return.py`, then re-run the sample load and watch the two windows.
2. Drive the checksum service once in `sharc-ref` (`DM(0x93DF6) = 1`, `DM(0xB6510) = 1`, a table, `DM(0xB650F)` set to the pitch window) to confirm that it never stores into the buffer; reading shows it does not.
3. `ns` not a multiple of 32 and the loop-end call stay open (section 1); neither depends on this hunt.
