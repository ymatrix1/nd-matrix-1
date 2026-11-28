# Synth SUSTPED: the firmware default and the per-program bit

- **Checked:** 2026-11-15 (OS v1.66).
- **Sources (local, never committed):** `docs/research/arm-edit-params.csv` (firmware edit-parameter table, row 307), `docs/research/data/synth-edit-messages.csv` (ARM captures of id `0xF8`), `tools/ns4_fw_order_check.py` (firmware field order of the `0x32` Layer buffer), the factory backup `Nord Stage 4 Factory Restore RevK v1.66.ns4b` (384 programs) read by `tools/synth_sustped.py`, the manual (v1.6X, p. 28), [grand-pad-leftovers](grand-pad-leftovers.md).
- **Question:** is the sustain pedal on for the Synth layers by default and per program (D93 left it open), so that `--synth-sustain-pedal` can be judged for promotion (D22)?

Tags follow `.agents/rules/provenance-in-code.md`. The ARM program-load run (about 4 minutes of boot per program) was not repeated: the flag has no DSP message, so the evidence is the static firmware table plus the program bits.

## 1. Answer

| Item | Result | Status |
|---|---|---|
| Firmware default | `Edt_Syn_kSustainEnable` (id `0xF8`, domain `0x11`) range 0..1, **default 1**; piano `0x150` default 1; organ `0x164` default 0 | VERIFIED (firmware table) |
| Program storage | 1 bit at block offset **+71** of each Synth layer block (after `kPStickEnable` 66 and `kPitchStickRange` 67..70, before `kVibMode` 72); blocks at section byte 327, bits 248 / 656 / 1064 | VERIFIED (order, width: `ns4_fw_order_check.py`), EMPIRICAL (position, below) |
| Factory programs | synth A set in **381 of 384**, B and C in **383 of 384**; the 5 programs off the default are `Cathedral Pipes`, `Wormhole Organ`, `J3 Split Special` (A off), `Forever Split` (B off), `ScissorSplit Whl` (C off). Grand Pad, Horizon Whl, Leaky Stabs II: 1, 1, 1. Per-program numbers: [data/synth-sustped-programs.csv](data/synth-sustped-programs.csv) | VERIFIED (run: `python tools/synth_sustped.py`) |
| DSP side | silent: the id sends identical words for 0 and 1 (`synth-edit-messages.csv`, words `0xE02E8/9`), so the hold is ARM-side note logic | VERIFIED (capture) |
| Engine before | `PedalState::sustped` of the synth layers false; a program's bit was ignored | |
| Engine now | the bit is decoded (`cbin/synth_sustain.rs`, `GlobalProgramDecoded::synth_sustain`, serde default on) and, under the opt-in `--synth-sustain-pedal`, applied to the three synth layers at program load; without a program the opt-in uses the firmware default (on) | |

## 2. Reading and verdict for the promotion (D22)

1. The hardware default is **on** and Grand Pad stores 1: the demo clip was played with the synth layers on the pedal. The D93 reading (hardware default unknown) is answered: `--synth-sustain-pedal` is the hardware behaviour for the factory programs.
2. D22 is still not met: the ARM note logic that holds a synth note on the pedal (the consumer of the `0xF8` flag) was not traced; the clip comparison is mixed (Grand Pad tail 6.7 to 10.9 s against 13.4 s, hold and release closer, but the spectral score 1.42 to 1.68 and width 0.46 to 0.40: the held pad piles up below 130 Hz). The default render stays unchanged; the opt-in now follows the program bit.
3. The position evidence is consistency, not a firmware read: a wrong position would give a random bit, not 99 percent ones equal to the default.

## 3. Load-path check on the real bank (2026-11-28, issue #678)

`cargo test -p nord-core --release --test real_bank_sustped` (needs the factory backup in `.reference/` or `NS4_FACTORY_BACKUP`; skips cleanly without it) loads all 384 factory programs of `Nord Stage 4 Factory Restore RevK v1.66.ns4b` through `Stage4Engine::apply_program` with `synth_sustain_program` on. Result: `pedals.sustped[4..7]` equals the decoded program bit for every program; the counts are 381 / 383 / 383 set and the five exceptions above are the only zeros. `python tools/synth_sustped.py` on the same file gives the same counts. VERIFIED (run). The OS v1.68 update was downloaded too, but the ARM harness addresses are v1.66 ones (`os-update-input.md`), so the ARM hold trace (follow-up 1) is still open.

## 4. ARM consumer of `0xF8`: first trace (2026-11-28, OS v1.66)

Static scan of `bank0_CODE.bin` (v1.66, `tools/arm_xref.py dis`, capstone, one throw-away scan script, no firmware bytes kept). VERIFIED (disassembly):

| Item | Result |
|---|---|
| Subscription | the layer class constructor `0xC00166EC` subscribes its object to id `0xF8` with `0xC00D57F0(this, 0xF8)` at `0xC00168D0` (260 subscribe call sites in the image; `0xF8` once) and registers it again through `0xC00F05C4(this, 0xF8)` at `0xC0016BCC` |
| Readers | two event handlers read the value with the store getter `0xC00D04D0` (id in `r2`): `0xC011D53C` at `0xC011E054` and `0xC0113758` at `0xC01138CC`. Both turn it into the boolean `value == 1` (`sub 1; clz; lsr 5`) next to `0xF6` (read the same way) and `0xF7` (a 0..9 switch), then go on to build layer configuration flags |
| Pedal path | not found yet. The booleans are consumed inside these layer-configuration handlers; the note-off deferral (where a released key is kept while the pedal is down) is a later consumer of those flags and was not reached |

So the flag is read by the ARM layer logic (matches the silent DSP side of s1), but the hold semantics (release timing, retrigger of a held key, pedal-up release) are still not confirmed against the firmware. Next step: run the edit-capture harness (`tools/arm_edit_capture.py`, needs the settled snapshot) with id `0xF8` at 0 and 1, diff the layer object memory to find the field the flag lands in, then find that field's readers in the note-off path.

Run results (2026-11-28, `tools/arm_edit_capture.py` machinery, `tools/arm_keydown.py` rig, v1.66; derived numbers only):

| Experiment | Result | Status |
|---|---|---|
| `Edt_Syn_kSustainEnable` 0 vs 1 on each Synth layer store, memory diff of the settled emulator | per layer exactly two bytes flip 0 to 1: `layer + 0x5EB8 + 2 (k - 4)` and its mirror `+ 0x290` (`k` = layer index 4..6), written by the change-detecting per-layer byte-table setter `0xC00C801C` (it compares the new byte with the old one and returns when equal); the rest of the diff is the value cell, scratch and stack | VERIFIED (run) |
| Note On, CC64 down, Note Off, 60 ticks, CC64 up through the real MIDI parser (UART2), flag 0 vs 1 | DSP messages identical in every phase. Inconclusive: in the settled state without a program only the piano path runs (`piano_key_event` fires, `synth_voice_event` `0xC01EC690` never does), so the flag cannot show an effect | VERIFIED (run), no conclusion |

Open: the same test needs a recalled program with an active Synth layer (`tools/arm_program_load.py` puts a factory `.ns4p` into the EEPROM image and boots the full machine, see `tools/arm_program_synth_velocity.py`), then Note On / CC64 / Note Off on UART2 and a diff of the Synth voice messages for flag 0 vs 1, plus the five exception programs.

## Implications

- `--synth-sustain-pedal` can be promoted once the ARM hold logic is traced and a second clip with the pedal and a synth layer agrees.
- The five exception programs are the check for the trace: their synth layer must not hold.
- The piano (default 1) and organ (default 0) bits are not decoded by this change.

## Follow-ups

1. Trace the ARM consumer of `0xF8` (the synth note-off path) in the harness and confirm the hold on the five exception programs.
2. A clip with a synth layer, the pedal and a program whose synth bit is 0.
