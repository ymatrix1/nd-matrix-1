# Research: the B3 Leslie install hunt, round 3 (OQ42, D190)

- **Checked:** 2026-11-21
- **Sources (local, never committed):** `target/fw_unpacked/bank0_CODE.bin` (ARM, OS v1.66), PNO core 1 image `target/PNO/core1_sram.bin`, boot log `target/bulk/pno_core1.script`, factory program `B3 Chords n Solo.ns4p` (`rotary_enabled` true, speed Slow, drive 2.44), harness `tools/arm_dsp_live.py` (`--return-path`), `tools/arm_leslie_paths.py`, `sharc-sys`, `sharc-disasm`. Committed data: `docs/research/data/org-system-load-pno1.script`, `org-system-key8-pno1.script`. New tool: `tools/msg_script_scan.py`. Numbers and addresses only; no firmware, sample or audio content is printed or committed. Builds on [b3-leslie-chain-link](b3-leslie-chain-link.md) (D171), [b3-leslie-feed](b3-leslie-feed.md) (D172), [leslie-locate](leslie-locate.md) (D139), [leslie-gate](leslie-gate.md) (D148), [leslie-paths](leslie-paths.md) (D155), [arm-dsp-live-link](arm-dsp-live-link.md) (D138), [arm-part-activation](arm-part-activation.md), [audio-remaining](audio-remaining.md) row 9.
- **Question:** Which ARM word installs the B3 Leslie chain element on PNO core 1 (head or element routine word, the four gain words `E-28..E-25`, the routine pointer layout), so that the oracle can run the stage the way the firmware does and measure whether B3 mid +3 dB / brilliance +5 dB close? If it closes, port an opt-in chain Leslie to `nord-core`; if not, narrow OQ42.

Tags follow `.agents/rules/provenance-in-code.md`: **VERIFIED** = reproduced by a command under Reproduce, **EMPIRICAL** = measured on this host, **ASSUMED** = reading without a source.

## 1. Short answer

**Negative, narrowed again. No ARM word installs the Leslie element, in the boot, the recall, the idle, key and off phases of a live ARM + PNO core 1 run with the DSP read-back served. The element words `E-28..E-25` are therefore not obtainable from the ARM stream, no firmware-faithful oracle run is possible, row 9 cannot be measured, and nothing is ported to `nord-core`.** The sharpest new fact: the ARM does write the Leslie's *internals* (rotor words, the two cabinet callee rows `DM(0xB2F29 / 0xB2F2B) = 0x1237F8`, the ring pointers, the drive words) but never its entry pointer, a head, or any word of the chain region `0xC2E00..0xC3100`.

| # | Check | Result | Tag |
|---|---|---|---|
| 1 | Live ARM + PNO core 1 (`arm_dsp_live.py --return-path`), `B3 Chords n Solo`, 800 load + 100 idle + 1,500 key + 300 off ticks | 2,441 DSP passes; 30,067 ARM words to the chip (7,340 in the load phase, 3 idle, 180 key, 155 off). `0x1256AD` and `0x1237F8` execute in **0 of 2,441** passes; `0x121E2B` (B3 processor, control) in 2,143 and the chain loop `0x1213E9` in 2,441 | VERIFIED (run, Reproduce 1) |
| 2 | Chain region stores | **0** ARM words with DM address in `0xC2E00..0xC3100` (the element records, the four heads `0xC2F48 / 0xC2F82 / 0xC2FBC / 0xC2FD6` and the chain pair `0xC2E06`) in the whole log, and 0 in the committed 7,340-word recall stream. So neither a head write nor a write of any `E-28..E-25` word exists | VERIFIED (`msg_script_scan.py region`) |
| 3 | Code pointers the ARM sends to PNO core 1 (boot log plus recall) | 12 distinct values in the code window `0x120000..0x128000`: `0x120552`, `0x121B33`, `0x121BBA`, `0x121BBE`, `0x121E2B` (slots), `0x122A71`, `0x122B85`, `0x1235AA`, `0x1236F8`, `0x1237F8`, `0x1250CF`, `0x125371`. `0x1256AD` is not one of them, raw or as code record `0x28495AB4` | VERIFIED (`msg_script_scan.py codes`, `value 0x1256AD`) |
| 4 | What the ARM does write for the Leslie | Boot: `DM(0xB2F29) = 0x1237F8`, `DM(0xB2F2A) = 0xE359E`, `DM(0xB2F2B) = 0x1237F8`, `DM(0xB2F2C) = 0xE31A6` (callee rows and ring pointers of the stage's state block), `DM(0xB2F27 / 0xB2F28)`. `DM(0xB2F25 / 0xB2F26) = 0x22B00 / 0x26000` (slow rotor / horn words) again and again (ticks 77, 348, 600 of the committed recall stream). In this run the drive words `DM(0xB2F2D / 0xB2F31) = 0x3E79F3E8` (0.2441, the program's drive 2.44 / 10). `DM(0xB2F21)` (the rotor phase limit, 0 in the image, which freezes the rotor) is **never written** | VERIFIED (log scan) |
| 5 | DSP read-back of the rotor phase (the ARM reads `DM(0xB2F23)` through `0xC0066E64`, [arm-part-activation](arm-part-activation.md) s1) | With the return path serving reads from the live chips, the return source served **0 calls** in the 2,441 passes (`return_path.calls = 0`), so no read of `DM(0xB2F23)` was seen. The rotor state machine of the ARM that would settle on the phase and could write the limit word is not entered in this window; the D155 harness answered 0 and also never saw an install | VERIFIED (run) |
| 6 | The chain elements the DSP image starts with | The ARM image carries the PNO core 1 boot data as plain words (`bank0_CODE` offset `0x4898C0..0x489B00`): a run of identical **null elements** `{routine 0x12637B, input pair 0xC2E06, state 0 or 0xB52C7, next 0xC2E4A, E-28 = -0.01, E-27 = -0.5, E-26 = -0.5, E-25 = 0.5}` (the words D171 read from the oracle image), with record headers `{tag, system address of DM 0xC2E10 / 0xC2E2A / 0xC2E33 / 0xC2F3E, size 0x68 / 0x24 / 0x42C / 0x288}`. The loop gain `g = clip(E-27 + E-28, +-E-25) + E-25` ([b3-leslie-feed](b3-leslie-feed.md) row 1) is `0` for every template element: the whole chain is **muted and empty by construction**, and turning an element on takes a write of `E-27` / `E-28` plus its routine word. The ARM never makes either | VERIFIED (dump + arithmetic); reading of the data as the DSP boot image ASSUMED (it equals the oracle image words) |
| 7 | Edit sweep with a scan for **stores into the chain region** (Leslie, rotary, routing, amp and FX type ids on every owning store, 40 ticks each, [leslie-paths](leslie-paths.md) harness; D155 scanned only for the routine literals) | **Not run to completion.** The run was stopped after about 40 minutes of ARM time without output; D155's sweep (literal scan only) stays the evidence for the edit ids, and the chain-region scan of the edit phases is follow-up 4 | not done |
| 8 | Row 9 (B3 mid +3 dB / brilliance +5 dB) with real element words | **Not measurable.** There are no real element words, so the D172 hand link (guessed gain words, junk output, gap 17.8 vs 10.9) is still the only oracle Leslie on the chain. The dry lane numbers stay sub +27.54, mid +1.04, brilliance +15.41 dB, gap 11.02 dB; native `--leslie firmware` (D63, D94) keeps its presence +1.5 to +2.8 dB, mid +3.1 .. +3.8 dB, brilliance +4.5 .. +5.8 dB | carried from D172 / D94 |

## 2. What this says about OQ42

1. **The install is not an ARM word stream property of this firmware image and harness.** The ARM names the Leslie's state block and cabinet callees, which proves the Leslie object exists in the ARM, but it never sends the entry pointer, a head or an element word, in 2,441 live passes with the read-back served (rows 1 to 5). With D138 (2,441 live passes without the return path), D139 (recalls) and D155 (7 recalls, 2,966 edit phases each, literal scan for the routine) the recall, the edit ids (literal scan only), the live passes and the served read-back are all closed for the Leslie entry.
2. **The element words are not a separate unknown from the install.** The templates are muted (row 6), so even a correctly linked element would be silent until the ARM writes the ramp words; and in the hardware the Leslie must produce the rotary seen in the clip ([leslie-gate](leslie-gate.md) row 4: 0.77 Hz and 7.0 Hz envelope lines). So some write turns an element on that this harness never produces.
3. **Candidates that remain** (none is tested here): (a) the install is DSP side, run from state the boot image sets up in hardware but that `core1_sram.bin` plus the boot log does not carry (the 5,910 L2 code words of the boot, D138, and the SPI block download `0xC0240548` are not modelled in the oracle's L2); (b) the ARM rotor / part state machine needs an input the harness never gives (a speed switch or pedal, organ on / off through the real MIDI / panel path), which row 5 suggests because its DSP read-back is never reached; (c) the rotary runs on another chip than the one that holds the Leslie state block (D135 found no FX evidence, but run-time loaded FX code is untested).
4. **No change to the engine.** `--leslie firmware` stays opt-in as D63 / D94 / D148 measured it, and the engine rotary is unchanged. The port the task allowed on a positive result is not made.

## Implications

- OQ42 narrows from "what installs the element words, state block and ring" to "what, outside the ARM word stream of a recall and its edits, enables a chain element": the state block is already known to be filled by the ARM (row 4), the head, routine word and ramp words are not.
- A future positive experiment needs one of: the L2 code of the boot in the oracle (candidate a), a real input to the ARM rotor state machine (candidate b), or a second hardware clip with a held fast speed to see whether the modulation depth follows the ARM speed words at all (so the ARM side is proven involved).

## Follow-ups

1. Candidate (a): run the live link with the boot's 5,910 L2 words loaded into the oracle's L2 image (`target/PNO/core1_l2.bin`, `pno_l2.script`) and re-probe `0x1256AD`; check whether any L2 routine writes the chain region.
2. Candidate (b): drive the real input paths of the ARM rotor state machine (UART MIDI CC for the speed switch, the organ on / off panel path) in the live link with the read-back served, and watch for the first `DM(0xB2F21)` store.
3. If either shows a write into `0xC2E00..0xC3100`, repeat the D172 rows 3 to 5 with the real words (`tools/dsp_lane_vs_hw.py`) and only then consider the opt-in port.

4. Re-run the D155 edit sweep (one B3 program is enough, it ran longer than 40 minutes here) and scan its message rows for stores into `0xC2E00..0xC3100`, to close the edit ids for the element words as D155 closed them for the routine literal.

## Reproduce

```bash
M=<main checkout with target/ and .reference/>; C=$M/target/fw_unpacked; R=$M/target/release
# 1. live link, return path served (about 100 s; needs the local firmware)
uv run --with unicorn --with capstone python -u tools/arm_dsp_live.py $C/bank0_CODE.bin $C/bank0_SRAM.bin \
  --eeprom $M/target/oled/eeprom.bin --program "$M/factory_programs/Program/Bank A/B3 Chords n Solo.ns4p" \
  --dsp-image $M/target/PNO/core1_sram.bin --live $R/sharc-live --return-path --images $M/target/PNO \
  --probe 0x1256AD --probe 0x121E2B --probe 0x1213E9 --probe 0x1237F8 --log live --out live.json
# rows 2 to 4 on the chip log and on the committed recall stream (no firmware needed for the second)
python tools/msg_script_scan.py live/PNO1.script region 0xC2E00 0xC3100        # 0 stores
python tools/msg_script_scan.py live/PNO1.script codes                         # 12 values, no 0x1256AD
python tools/msg_script_scan.py docs/research/data/org-system-load-pno1.script region 0xC2E00 0xC3100
python tools/msg_script_scan.py docs/research/data/org-system-load-pno1.script value 0x1256AD
# row 6: the template elements in the ARM image (offset 0x4898C0..0x489B00 of bank0_CODE.bin, 16 bytes per line)
python - <<'PY'
import struct
d = open("target/fw_unpacked/bank0_CODE.bin", "rb").read()
for o in range(0x4898C0, 0x489A40, 16):
    print(hex(0xC0000000 + o), *("%08x" % x for x in struct.unpack("<4I", d[o:o + 16])))
PY
# row 7 (not completed here): edit sweep; the gzip of message rows it writes can be scanned for the chain region
uv run --with unicorn --with capstone python -u tools/arm_leslie_paths.py $C/bank0_CODE.bin $C/bank0_SRAM.bin \
  --eeprom $M/target/oled/eeprom.bin --out sweep "$M/factory_programs/Program/Bank A/B3 Chords n Solo.ns4p"
```
