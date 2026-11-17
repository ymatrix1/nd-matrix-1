# Research: what makes the firmware Leslie `0x1256AD` run? (not found; PNO core 1 stream of two B3 programs; D139)

- **Checked:** 2026-11-17
- **Sources (local, never committed):** `target/fw_unpacked/bank0_CODE.bin` (ARM, OS v1.66, base `0xC0000000`), PNO core 1 SRAM image `target/PNO/core1_sram.bin`, ARM boot log `target/bulk/pno_core1.script`, two new full recalls of `B3 Chords n Solo.ns4p` and `B3 Jazz Split.ns4p` (`tools/arm_dsp_capture.py one`, phases `load` 800 ticks, `idle`, `k36 / k60 / k84`), `sharc-sys` (`--pno --epdma --probe`), `sharc-disasm`. New tools: `tools/arm_table_watch.py`, `tools/arm_capture_script.py`. Numbers and addresses only; no firmware, sample or audio content is printed or committed. Builds on [b3-leslie-pno-lane](b3-leslie-pno-lane.md) (D126), [b3-leslie-fx-chip](b3-leslie-fx-chip.md) (D135), [fx-leslie](fx-leslie.md), [fx-chain-install](fx-chain-install.md), [pno-module-chain](pno-module-chain.md), [arm-part-activation](arm-part-activation.md), [audio-remaining](audio-remaining.md) row 9.
- **Question:** Which ARM message, DMA or flag makes the PNO core 1 chain loop call the Leslie processor `0x1256AD` (and its callee `0x1237F8`)? Can the harness open that gate, and if it does, how does the Leslie output compare with native `organ_b3` and the hardware clip (sub +27.54, mid +1.04, brilliance +15.41 dB, gap 11.02 dB)?

Tags follow `.agents/rules/provenance-in-code.md`: **VERIFIED** = reproduced by a command under Reproduce, **EMPIRICAL** = measured on this host, **ASSUMED** = reading without a source.

## 1. Short answer

**Not found. The gate was not opened, so there is no Leslie output to compare and nothing is ported.** Scope: the PNO core 1 messages of two B3 programs (`B3 Chords n Solo`, `B3 Jazz Split`), an 800-tick recall plus idle and key phases, and three Leslie edit ids (0x189 enable, 0x18a drive, 0x18d speed; 0x18c stop was not edited). Not tested: rotary on through the organ type or the FX on/off path, amp type 7 "To Rotary", organ type changes after recall, the rotor speed pedal or switch input, runs longer than 800 ticks, messages to ORG or FX cores. What was tested, exactly:

| # | Check | Result | Tag |
|---|---|---|---|
| 1 | Who can call `0x1256AD` on PNO core 1? | Only an indirect call. No instruction names it (no `call 0x1256ad`, no immediate `0x1256AD` in `0x120000..0x127400`, listing of 12,267 lines tiles exactly). The chain loop `0x1213E9` (unrolled, four heads `DM(0xC2F48 / 0xC2F82 / 0xC2FBC / 0xC2FD6)`) loads `I1 = DM(head)`, `I8 = DM(-32, I1)` and executes `CALL (M8, I8)`; the Leslie would have to be the routine word of a chain element. Its own callee is the row `DM(0xB2F29 / 0xB2F2B)` (`0x24B532..0x24B54E`, `I8 = DM(0xB2F29)`). | VERIFIED (`sharc-disasm` listing) |
| 2 | Does the ARM name `0x1256AD` anywhere? | Only through the channel table `0xC05450C8`, row 6 (`0xC0545128`, code record, `0x28000000 + 4 * 0x1256AD`), row 7 `0xC0545138` (`0x126367`). The rows 6 and 7 are referenced by two static descriptors in flash (`0xC02BB77C`, `0xC02BB8A4`), each next to a RAM object pointer (`0xC0547C18`, `0xC0547C28`), never by a code literal. | VERIFIED (`tools/arm_xref.py literal`) |
| 3 | When does the ARM read row 6? | **Once, during the boot, never in a recall.** A read hook over the table `0xC05450C8..0xC0545148` (`tools/arm_table_watch.py`) during boot and 800 recall ticks of `B3 Chords n Solo`: boot has 32 accesses (12 reads of rows 0..5 by the table scan `0xC01D5C1C`, and 2 by the lookup `0xC0013ca8..0xC0013ce0` that reaches row 6); the recall has **0 accesses**, row 6 included. The one lookup is the channel resolver `0xC0013928` called from `0xC0220DB0` (`r0 = 0xC02BB77C`, `r1 = 2` = PNO core 1, `r2 = 1`, `r3 = 0`): it binds the row to an object; nothing sends it afterwards. | VERIFIED (hook run) |
| 4 | Does a B3 recall send `0x1256AD` or any other Leslie routine to a DSP? | **No.** The `load` phase of both programs sends 7,340 messages to PNO core 1 (tick 0 to 800). No word is `0x1256AD`, `0x1237F8`, `0x1236F8`, `0x125371`, `0x1235AA`, `0x1250CF` or `0x126367` except the six stage-record words `DM(0xB0479..0xB047B) = 0x1250CF, 0x125371, 0x121BBA` and `DM(0xB0481..0xB0483) = 0x1250CF, 0x1235AA, 0x1236F8` (tick 178, ARM `0xC021D02C / 0xC021CED0 / 0xC021CD74`, identical for both programs). The slot installer sends `DM(0xC01D6..0xC01F2)` = `0x122A71, 0x121BBE, 0x121B33 x2, 0x122B85 x2, 0x121E2B x2` (the B3 processor `0x121E2B` twice, no Leslie). No write to a chain head `0xC2F48..` occurs. | VERIFIED (capture scan) |
| 5 | Replay of that recall into the PNO lane | Boot log `pno_core1.script` (an earlier ARM run than the new recall; the two are not from one ARM session) + the 7,340 PNO core 1 `load` messages + the `idle` and `k60` messages at pass 16 (`tools/arm_capture_script.py`), `sharc-sys --pno --epdma --passes 300 --probe 0x1256AD`: **executed in 0 of 316 passes** (same as D126, now with the real recall). Positive control, same replay: `--probe 0x121E2B` (B3 processor, sent by the slot installer) executes in **316 of 316**, so the probe reports hits; `0x1250CF` is 0 of 316. DSP read-back is answered by the stub `0xC0066E64` (0) in the capture run. The result says these messages cannot open the lane, not that the hardware cannot. | VERIFIED (run) |
| 6 | Do the Leslie edit ids open it? | See section 3: the organ FX unit accepts the edits 0x189 (enable), 0x18a (drive), 0x18d (speed), the ARM sends only the rotor speed words and drive words `DM(0xB2F25 / 0xB2F26 / 0xB2FA1 / 0xB2F2D / 0xB2F31)`; **no routine pointer in the 120 ticks run after each edit**. | VERIFIED (run, 120 ticks per edit) |
| 7 | Leslie output vs native `organ_b3` and the clip | **Not measurable**: `0x1256AD` never runs in the lane, so the band numbers stay the D126 lane pair (sub +27.54, mid +1.04, brilliance +15.41, gap 11.02 dB). | n/a |

## 2. What the data say about the gate

- The B3 recall installs two **stage record sets** on PNO core 1 (`0xB0476..0xB047D` and `0xB047E..0xB0485`): each set holds the shared first stage `0x1250CF` plus either `0x125371` (nonlinear resonant filter) and `0x121BBA`, or `0x1235AA` (table LFO) and `0x1236F8` (cabinet). These are Leslie *family* routines, but **none runs** (D126, row 5 above: 0 of 316 passes for `0x1250CF`, `0x125371`, `0x1235AA`, `0x1236F8`). Nothing in the PNO listing names `DM(0xB0476..)`; the sets are read through a pointer, like the FX chain links `DM(0xB0473) = 0xB0471` the same recall sends to the FX chip.
- Because no PNO instruction names a `0xB04xx` word and the executed pcs of the lane stop at the slot routines and the chain loop, the consumer of the stage sets is a **chain element or module list that is not linked into any head** (`DM(0xC2F48..)` keep the null stub `0xC2E4A`, routine `0x12637B`). The ARM never writes a head in the recall (no `0xC2F48..0xC2FD6` word in 7,340 messages).
- The ARM object that owns row 6 is created at boot only (resolver call, `r1 = 2`). The hook has a positive control (32 boot accesses on the same range), so the 0 means something. Its later use would read `0xC0545128` again; the hook saw 0 reads in 800 recall ticks and in the edit runs. So the install of `0x1256AD` is **not a step of the program recall, the Leslie enable, speed or drive edit** as the harness runs them.
- Hypothesis (ASSUMED, not shown): a chain-head write or element link is what installs the Leslie; the chain loop reading heads while the recall installs stage records are two different structures. The install may be triggered by a part command that this harness never drains (`[part + 0xC8]` list, [fx-contexts](fx-contexts.md) s7, [arm-part-activation](arm-part-activation.md) follow-up 1), or by a DSP read-back handshake that the stubbed SPI master `0xC0066E64` answers with 0.

## 3. Edit replay (Leslie ids on the organ FX unit)

`tools/arm_table_watch.py --edit ID=VALUE` finds the organ FX units with `tools/fx_clients.py` (units 3 and 9, `kind organ`), writes the value cell and dispatches `0xC00CC4F0(unit, 0, id, &value)` after the 800-tick recall, then runs 120 ticks. Module rows of the units are the state-table rows `0xA / 0x14` (unit 3) and `0xB / 0x15` (unit 9), not code rows.

| Edit | Messages in the call | Messages in 120 ticks | Row `0xC05450C8..` accesses |
|---|---:|---|---:|
| `0x189` (Leslie enable) = 0, then 1 | 0 | `0xB2F26`, `0xB2FA1`, `0xB2F25` (speed words, same as idle) | 0 |
| `0x18d` (speed) = 1 | 0 | the same three speed words | 0 |
| `0x18a` (drive) = 100 (`--q16`) | 2 | `DM(0xB2F31)` = `DM(0xB2F2D)` = `0x3F499326` (0.7874 = 100 / 127, the D98 law); then the three speed words | 0 |

The drive words are the D98 words (VERIFIED again by this run); the enable id sends nothing, as in [fx-contexts](fx-contexts.md) s7.

## 4. Implications

1. The firmware `0x1256AD` is **not installed by the PNO core 1 messages of two B3 recalls or by the edit ids 0x189, 0x18a, 0x18d (120 ticks each)** in this harness; the limits above (rotary-on paths, amp type 7, longer runs, other cores, pedal input) are untested. The chain-head write is a hypothesis. D126 and D135 stay as written; this adds the real recall, the edit ids and the row-6 reader trace.
2. Hypothesis (ASSUMED): the chain loop reads heads, not stage records; the stage-record sets `0xB0476..` and the chain heads look like two structures, and only the first is installed by the recall.
3. No native port. The native Leslie (opt-in `--leslie firmware`, D63) is unchanged; no default moves.

## 5. Follow-ups

1. Run the part command consumer (`[part + 0xC8]` list; the node handler of `0xC010E774`) with the DSP read-back stub `0xC0066E64` answered from the oracle; then watch `0xC0545128` and the chain heads `0xC2F48..`. This is the only path left that can create the element.
2. Search the ARM image for writers of the four chain-head words (ASSUMED: table `0xC0546F28` rows 20..23, per [fx-chain-install](fx-chain-install.md); the rows were not re-checked here) and run each with a synthetic element (routine `0x1256AD`, gain 1.0) to learn the element layout the loop expects; compare the lane against the clip only after the element is linked and its input pair is the organ bus.
3. Re-run `sharc-sys --probe 0x1256AD` and `tools/dsp_lane_vs_hw.py --segment organ_b3` after any of the above; port only with VERIFIED provenance.

## Reproduce

```bash
# row 6 reader trace (boot + recall) and edit replay; about 2 min each
C=target/fw_unpacked
uv run --with unicorn python tools/arm_table_watch.py $C/bank0_CODE.bin $C/bank0_SRAM.bin \
    --eeprom target/oled/eeprom.bin --range 0xC05450C8:0xC0545148 \
    --edit 0x189=0 --edit 0x189=1 --edit 0x18d=1 \
    "factory_programs/Program/Bank A/B3 Chords n Solo.ns4p"
# lookup caller: add --call 0xC0013928 --call-r0 0xC0545128 (prints r0..r3, lr, stack words)
# full recall capture, then a replay script for the lane
uv run --with unicorn python tools/arm_dsp_capture.py one $C/bank0_CODE.bin $C/bank0_SRAM.bin \
    --eeprom target/oled/eeprom.bin --out cap "factory_programs/Program/Bank A/B3 Chords n Solo.ns4p"
python tools/arm_capture_script.py cap/Bank_A__B3_Chords_n_Solo.txt.gz --chip PNO --core 1 --phase load > load.script
sharc-sys target/PNO/core1_sram.bin --pno --epdma --passes 300 --script-flat target/bulk/pno_core1.script \
    --script-flat load.script --probe 0x1256AD     # executed in 0 of 316 passes
```
