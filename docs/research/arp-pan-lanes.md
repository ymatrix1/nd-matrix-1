# Arp step pan is the lane select of the synth key word (OQ40)

- **Checked:** 2026-11-20
- **Sources (local, never committed):** ARM image `target/fw_unpacked/bank0_CODE.bin` + `bank0_SRAM.bin` through the Unicorn harness; PNO core 2 image `target/PNO/core2_sram.bin` through `sharc-sys` / `sharc-live`; `factory_programs/Program/Bank F/Synth Engine.ns4p` (from `.reference/Nord Stage 4 Factory Restore RevK v1.66.ns4b`). Committed numbers: [arp-pan-lanes.json](data/arp-pan-lanes.json). Tools: `tools/arm_key_words.py`, `tools/arp_pan_lanes.py`.
- **Question (OQ40):** why does the ARM start the Synth Engine voice (arp zigzag, custom 14 step pattern) with only bank bit `0x01` in the key word `0x0100E200`, so that the DSP voice window stays silent (both bits: peak 0.194), and does the hardware sound that step?

## 1. Answer

**The hardware sounds the step. The two low bits of the key word are not voice banks: they are the output lane select of the voice record, and the lane select is the pattern pan of the step.** Synth Engine's pattern pan is `CRLCRCLCCRLRCC` (`tools/ns4_arp_patterns.py`); the ARM plays steps 1.. as R, L, C, R, so the first key word is right only (`0x01`). The voice window `0xC1F89:4:2` of the chain census reads the **even words** of the interleaved voice record, which is the left lane, so a right step reads 0.0. No firmware oddity, nothing native was missing for the note; the native engine's hard pan gain was wrong (3 dB, section 4).

| Step | Evidence | Tag |
|---|---|---|
| The builder `0xC01EC690(this, hdr, rec, kind)` is called twice per step (kinds 2 and 4, flag words `2` and `4` of the table at `[0xC02FD3C8 + 8 kind]`), from the note-on tail `0xC01E849C` / `0xC01E84C0`; it posts one record `{note 60, 0x10000, pan code, 0}` and does not choose the lane bits | trace of `0xC01EC690`, `0xC01E72C4`, `0xC01E81DC` (hooks, key 60, 0.9 s) | VERIFIED (run) |
| The note-on `0xC01E81DC(obj, count 128, r2 0, r3 = pan code)` is entered from `0xC0123B00` (Glue listener) and `0xC0114D5C`; `r3` runs 2, 1, 0, 2 over four steps = pattern pan R, L, C, R (code `0 C`, `1 L`, `2 R`, `ns4_arp_patterns.PAN_CHARS`) | same trace, `ns4_arp_patterns.decode_pattern` on the program | VERIFIED (run) |
| The key word store (`pc 0xC01D0704`, port write) carries bank bits `3 - code`: `0x01`, `0x02`, `0x03`, `0x01` at DM words `0xC1DCC`, `+1`, `+2`, `+3` (the ARM advances the word per step); the off word `0x0000E200` follows 3,104 ticks later | port write hook, 4 steps | VERIFIED (run) |
| Voice record `DM(0xC1F89 ..)` of PNO core 2: even words = left lane, odd words = right lane. Replay of the recorded load stream plus one key word at DM `0xC1DCC` (1,500 passes): flags `0x01` left 0.0 / right 0.1947, `0x02` left 0.1947 / right 0.0, `0x03` both 0.1947. A lane that is on carries the same level as the centre voice: a switch at unity | `tools/arp_pan_lanes.py`, [arp-pan-lanes.json](data/arp-pan-lanes.json) | VERIFIED (run) |
| Live chain, key held 8,000 ticks (first step, pan R): window `0xC1F89:8:1` peak **0.1944**; the old window `0xC1F89:4:2` peak 0.0 | `tools/arm_key_words.py`, `--chip PNO:2=org=...=0xC1F89:8:1` | VERIFIED (run) |
| The 3 steps of a 14,000 tick run (flags `0x01`, `0x02`, `0x03`) write DM words `0xC1DCC`, `0xC1DCD`, `0xC1DCE`: other voices than the one of the window (record offsets unread), so only step 1 and 4 land on the window voice | run, port writes | EMPIRICAL |

Earlier readings in [silent-programs](silent-programs.md) s6 ("voice bank bit", "round robin banks", "both bits: peak 0.194") are explained: poking `0x02` added the left lane, which the window read.

## 2. Reproduce

```bash
M=C:/dev/nord-stage-4-emulator   # main checkout with target/ and factory_programs/
uv run --with unicorn --with capstone python tools/arm_key_words.py 1 key_words.json $M/target/fw_unpacked/bank0_CODE.bin \
  $M/target/fw_unpacked/bank0_SRAM.bin --eeprom $M/target/oled/eeprom.bin --program "$M/factory_programs/Program/Bank F/Synth Engine.ns4p" \
  --live $M/target/release/sharc-live.exe --poll-advance 64 --key-ticks 14000 --chip "PNO:2=org=$M/target/PNO/core2_sram.bin=0xC1F89:8:1"
python tools/arp_pan_lanes.py key_words.json $M/target/PNO/core2_sram.bin target/release/sharc-sys   # lane peaks for pan C, L, R
```

## 3. Consequences for the census

`tools/program_rank_chain.py` now reads `0xC1F89:8:1` (both lanes). Peaks of programs with centre or left voices are unchanged (same lane peak); only a program whose only sounding steps are right-panned changes from `0.0` to a level. The 384-program census was **not** rerun: the `synth_chain_silent` flag of Synth Engine in `program-ranking.csv` is stale until it is.

## 4. Native: hard pan gain

`pan_gains` (`synth/pattern.rs`) used a constant power law with `sqrt(2)` on the pointed side (ASSUMED, "hardware pan law unmeasured"). The voice record shows a lane switch at unity: centre `(1, 1)`, left `(1, 0)`, right `(0, 1)`. Native now uses that (`(1 - p).min(1)`, `(1 + p).min(1)`; the fade between the positions is ASSUMED, patterns only use `-1`, `0`, `1`). Per-step panned arp notes and Gate mode pan are 3 dB (a factor `sqrt(2)`) quieter on the pointed side than before. What is not measured: the layer pan and the FX and master stages behind the voice record; this change covers the pattern pan at the voice only.

## Implications

- OQ40 is closed: the hardware plays the step, the ARM key word is correct, the chain harness read one lane.
- The key word flag byte of the keyboard path (`0x0B`, both lanes) and of an arp step share one meaning: lanes. [keydown-zones](keydown-zones.md) s7 called them bank bits.

## Follow-ups

1. Rerun the 384-program census with the two-lane window (about an hour with 8 workers) and the `program-ranking.csv` chain columns.
2. Read the voice record offsets of voices 1 and 2 (DM words `0xC1DCD`, `0xC1DCE`) to confirm that the arp spreads steps over voices.
3. Measure the layer pan law (stored layer pan) against the voice record; only the pattern pan is covered here.
