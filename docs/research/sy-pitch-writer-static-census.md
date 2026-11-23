# SY pitch entry `e`, seventh hunt: static census of every ARM call site of the SY / SMP message writers (negative, 2026-11-22)

- **Checked:** 2026-11-22
- **Sources (local, never committed):** OS v1.66 `target/fw_unpacked/bank0_CODE.bin` (ARM mode, base `0xC0000000`), `target/DSP/dsp_image_3_SY1SY2.bin`, `target/oracle/sy2.lst`, [arm-chan-tables.csv](arm-chan-tables.csv), [arm-param-table.csv](arm-param-table.csv); new `tools/arm_sender_sites.py` and `tools/arm_const_track.py` (tests `tools/tests/test_arm_sender_sites.py`, `tools/tests/test_arm_const_track.py`). Follows [sy-pitch-entry-writer](sy-pitch-entry-writer.md) s1 to s10 (D105, D111, D112, D124, D141), [sy-flash-bus-hunt](sy-flash-bus-hunt.md) (D176), [sy-flash-service-hunt](sy-flash-service-hunt.md) (D196), [smp-page-refill](smp-page-refill.md) (D191, D199).
- **Question:** Six hunts drove the ARM and the SY chip through the paths a harness reaches and found no writer of the 16-bit entry at `DM(0x9354A + index)`. Statically, which ARM code **can** send a word to the SY at all, and can any of it address the pitch window `DM(0x93500..0x935A0)`?

Tags follow `.agents/rules/provenance-in-code.md`. Addresses and counts only; no firmware word, sample or audio value is quoted.

## 1. Short answer

| Question | Result | Tag |
|---|---|---|
| Is the ARM to SY / SMP link reached through shared writers? | Yes for path 1: `0xC0062E10` / `0xC0062E78 (comm, word1, word2)` (the path 1 block of every table sender is dead for targets 0 to 5, [arm-param-messages](arm-param-messages.md) s2). **313 call sites** of the two writers, each resolved with the linear constant tracker (`arm_const_track`): comm known at 64 (all comm 4 = SMP), unknown at 249 | VERIFIED (`python tools/arm_sender_sites.py bank0_CODE.bin`) |
| Does any call site name the SY with a constant comm? | **No.** 0 of 313 have `comm = 3`. The SY is reached only through a comm taken from a table record or a register | VERIFIED (same run) |
| Does any constant `word1` name the pitch window? | **No.** 22 sites have a constant `word1`; none lies in `0x93500..0x935A0` (21 are SMP words: `0x94213`, `0x9404A`, `0x9407E..0x94081`, `0x94088`, `0x9408B`, `0xB2031`, `0xB2270`, `0xB2271`, `0xB22C4`; `0` twice; the 22nd is `0x92533`, see s3) | VERIFIED (same run) |
| How are the 292 sites without a constant `word1` built? | 222 are `add r1, r1, #imm` (record base from the channel table plus a word offset: the table senders), 18 load `r1` from a record field (`ldr r1, [rec, #n]`, offset 0 of a record), 16 pass a register (`mov r1, rX` or a link / frame register), 24 compute `add r1, rX, rY` and 12 forward their arguments; the last 36 all lie in the SMP cluster `0xC00AD498..0xC00B7A3C` (s2) | VERIFIED (disassembly classification, scratch script) |
| How far can the SY table senders reach? | The SY channel rows (target 6, 97 rows in `arm-chan-tables.csv`) have bases `0x92196 .. 0x92E4E` (largest `0x92E4E`); the SY senders of `arm-param-table.csv` add word offsets up to `0x22` (`0xC0017088`: `0x7 0x8 0xA 0xE 0x1D 0x1F..0x22`, the others `0x4`, `0x8`). Largest reachable address about `0x92E70`, **`0x690` words below the window**. The row index is bounded by the 48 / 56 rows | VERIFIED (tables, CSVs); index bound ASSUMED |
| Verdict | **Writer not found.** The ARM has no sender that can reach `DM(0x9354A + k)`: the only way left is a computed address inside the SMP cluster applied to comm 3 (not shown, s2) or a path that is not an ARM message (flash read service, SPI DMA opcode 9). `--sample-pitch fw` stays opt-in; no default, golden or `render_hash` moves | open |

## 2. The SMP cluster `0xC00AB..0xC00BD` is a piano parameter and directory applier, not an SY sender

Its constant `r0` is 4 (SMP). Example `0xC00B29E8`: a 4 way switch on `[obj]` walks a table at `0xC02AE0EC` and sends words to `DM(0xB211B .. 0xB22C5)` (`0xB2000 + k` plus the loop index); the same family of addresses as the SMP directory `DM(0xB237B + i)` that the live SMP chip needs ([smp-page-refill](smp-page-refill.md) s2). So the ARM does write directory words to the SMP, by computed address; the SY offset table `DM(0xB2000..)` (zero fill, D176) has **no such writer in the code**: no call with comm 3, no table row, no register comm in the cluster. That is the asymmetry that D176 asked about: for the SY nothing in the ARM can fill `0xB2000..` or the window.

## 3. The one SY capable constant outside the tables

`0xC00C0B6C`: `r1 = 0x92533`, `r2 = 1`, `r0 = ip` (comm from a register): a one word SY write `DM(0x92533) <- 1`, an SY record word below the window (record base `0x92406 + 0x38 k`). Not the window.

## 4. D213 setter call graph (`0xC01C61F0`, Osc Env Amount)

One direct caller, `0xC0017F18`, inside the strip apply dispatcher `0xC0017088` (loop `0xC00173EC`). The setter itself calls only float helpers (`0xC0270BD4`, `0xC0270C80`) and the taper `0xC01CD540` (table `0xC02CA2C0`); it sends nothing, the dispatcher sends. The same dispatcher also holds the SY senders with offsets `0x7 .. 0x22` on the pair block base `0x9219E` (s1). So the D213 setter sits in the one routine that mixes ORG strip fields and SY voice words, and no edge reaches the window.

## 5. What was not repeated (and why)

| Angle in the task | Decision |
|---|---|
| Write watch on the window from the SHARC consumer backwards | Done four times with a positive control (D111, D112, D176, D196: `--watch 0x93500:0xA0`, vectors 0 to 31, 224 handler fills, ring refill, real stream). Nothing external feeds the DSP except the ARM messages and the DMA chains, both bounded above; a fifth run adds no information |
| ARM RAM diff before and after key-on across notes and programs | The ARM can only reach the SY through the senders of s1; the 545 + 19 per note messages already enumerate what it sends (D124: 119 of 119 programs). A RAM diff cannot show a message that no sender emits |

## 6. Caveats of the census

| Item | Status |
|---|---|
| Inlined path 1 senders (the function loads the comm object from `0xC053F8A4` and stores through `[obj + 0x10]` itself) | About 845 pairs (`ldr rX, [rN, #0x10]` followed by `str ..., [rX]`, 1,690 words outside the writers). They are the table senders of [arm-param-table.csv](arm-param-table.csv) (word offsets in the dual `(a, 4a)` immediates); this tool does not resolve them, the offset bound of s1 comes from that CSV |
| Comm of the 249 unknown sites | Not resolved; assumed to come from the channel record (`table[ch]`), which is the only source seen |
| Tracker | Linear, resets at every branch; values built across basic blocks are not seen (ASSUMED approximation) |
| The SY FIFO consumer in `sharc-live` | Open: the SMP chip replayed stale FIFO pairs until the index bias `DM(0x2400) - 0x80000` was fixed (D199); the SY model still stores the plain address and caps at `DM(0x92180)` ([smp-page-refill](smp-page-refill.md) s4). This does not change the ARM side census above, but any SY run that fed more than 16 pairs per ISR may have lost or replayed messages |

## 7. Remaining leads, ranked

| # | Lead | Why it is next | Cost |
|---|---|---|---|
| 1 | Real sample directory read through `0xC024C284` and the SMP style directory feed for the SY (`tools/arm_flash_model.py` answering `DM(0xB6000/1)` reads with the restore image) | The only ARM source of a computed address left; the SMP side shows the ARM does write directory words by computed address (s2) | high (needs the flash file system layout) |
| 2 | Fix the SY FIFO bias in `live_smp` style (`index_bias`, cap `DM(0x92419)` or `0x92180`) and re-run the SY census (D153 on) | A stale replay or a dropped pair would hide a message the ARM does send | medium |
| 3 | Verify the premise: does the hardware read `DM(0x9354A + k)` with a non zero value at all? (a hardware clip of one sample note against the oracle with `e` poked) | If a different path sets the step (`DM(0x9242D)`), `e` is not the quantity to find | medium (needs a recorded clip) |
| 4 | Resolve the comm of the 249 unknown sites by an interprocedural tracker | Closes caveat s6 | medium |
| 5 | SPI DMA opcode 9 with a flash model (`0x1C03AA`) | Never sent by the ARM in any run | high |

## Implications

- No ARM message sender can name `DM(0x93500..0x935A0)`; together with D105 (no packer), D112 (stream holds no pitch word), D141 (no DMA chain) and D196 (flash services land elsewhere) the entry is not written by anything the firmware images contain, in any path an emulator reaches. Either hardware state not present locally (flash content) fills it, or the premise (a non zero `e` is read in the hardware) needs a recording to check (lead 3).
- `--sample-pitch fw` stays opt-in; the model `step_join::table_entry_for_key` is unchanged.

## Follow-ups

Leads 1 to 3 of s7. Reproduce:

```bash
python tools/arm_sender_sites.py target/fw_unpacked/bank0_CODE.bin --list
python tools/sy_flash_service_hunt.py target/fw_unpacked/bank0_CODE.bin target/DSP/dsp_image_3_SY1SY2.bin target/oracle/sy2.lst
uv run pytest tools/tests/test_arm_sender_sites.py tools/tests/test_arm_const_track.py tools/tests/test_sy_flash_service_hunt.py
```
