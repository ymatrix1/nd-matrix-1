# ARM -> DSP event sweep over the 384 factory programs

Checked: 2026-11-23.

## Sources

- Tool: `tools/arm_dsp_event_sweep.py` (tests `tools/tests/test_arm_dsp_event_sweep.py`); derived data
  [data/arm-dsp-events.json](data/arm-dsp-events.json) (384 entries, numbers only, no firmware bytes).
- Run: the unpacked ARM firmware under the harness of [arm-dsp-capture.md](arm-dsp-capture.md) and
  the Master Clock position feed of `tools/arm_arp_events.py` (`arm_arp_events.Recorder`, 120 BPM).
  Chord 60, 64, 67 at velocity 100, held 1.0 s, released at 0.7 s; every ARM -> DSP message with
  its time in ms since the note-on (24 kHz timer counter, D130).
- Voice key word layout: `tools/arm_key_words.py`; gate words: PR 633 (Gate arp mode).

## Question

Which ARM -> DSP messages does each program's layer send while a chord is held, as derived events
(voice on / off, float gate edges, other repeating addresses), so the native engine can be diffed
against them?

## Method

`classify` reduces the message log per layer:

| Class | Match | Layer from |
|---|---|---|
| voice on / off | key words `DM(0xC1DCC .. +15)`; on = a lane bit (`0x03`) in the flag byte | `(dm - 0xC1DCC) % 3` -> A, B, C |
| gate edge | `DM(0xC1396/7)`, `0xC13E6/7`, `0xC1436/7`; recorded on a change of the stored float | address family A, B, C |
| other | any other address in the layer's 0x50-word block from its gate base | block (ASSUMED stride) |
| unassigned | every other address | none |

JSON entry: `{program, layers: {A|B|C: {voice_on_ms, voice_off_ms, gate_edges: [{t_ms, dm, value}],
other: {dm: count}}}, unassigned: {dm: count}}`. `unassigned` is an addition beside the agreed
schema; readers of `program` and `layers` can ignore it.

## Findings

| Finding | Tag |
|---|---|
| 384 of 384 programs boot and finish (0 failed). | VERIFIED (run, `batch.log`: `done, 0 failed`) |
| Distinct message classes found: voice key words (276 programs), float gate levels (4 programs), constant background words, other word families at `DM(0x9512E..)` and `DM(0x95331..)`. | EMPIRICAL |
| Gate edges occur in only 4 programs: Dual Gates (A 17, B 17, C 12 edges), FM Cloud Whl (B 28), Calling Whl (C 32), Time Warp Saw (A 10). Each edge writes the pair `0xC1396` and `0xC1397` (B: `0xC13E6/7`, C: `0xC1436/7`) in the same ms with the same level. In Dual Gates layer A the level is 0.0 at 83, 208, 458, 583, 708 ms and 1.0 at 124, 249, 499, 624, 999 ms: 41 ms low then 84 ms high, a 2/3 duty cycle (the 120 BPM step time is an input, ASSUMED, see `arm_arp_events`). | EMPIRICAL |
| In gate programs the chord goes out as one key word per layer note at 0 ms (Dual Gates: 2 on words at 0 ms per layer, 2 off words at 700 ms) and the arp does not send new key words; in the other arp programs each step sends on / off words (Boxxx layer A on at 0, 249, 749, 999 ms). | EMPIRICAL |
| 108 programs send no key word on any layer. 22 programs (B3 and similar organ programs, e.g. B3 Shade) add words at `DM(0x9513A..0x9515E)` in all 22 and 0x9512E..0x9515A in 19 of them; other programs add words at `DM(0x95331..)` (3 to 7 programs per address, family not identified). | EMPIRICAL |
| Constant background: `DM(0xB2F25)`, `0xB2F26`, `0xB2FA1` are sent 95 times in 1.0 s in 381 of 384 programs; they carry no layer or note information. | EMPIRICAL |
| 52 layer entries have different numbers of on and off words. The run ends at 1.0 s with release at 0.7 s, so arp offs after 1.0 s are cut off, and a chord note can start voices on a layer without an off word in the window. Consumers should compare on-times and not pair on / off counts blindly. | EMPIRICAL |
| Per-layer `other` is empty for all programs (no address in a layer block beyond the gate words). | EMPIRICAL |
| Time 0 ms is the first ms after the note-on; words at t = 0 include the chord's immediate voice starts. | ASSUMED (timer read before the first tick) |

## Implications

- Native-vs-ARM diffs should compare voice-on times per layer (`voice_on_ms`) for Poly and Arp
  programs and gate-edge times and duty for the 4 Gate programs (the 2/3 duty cycle and the single
  chord send match PR 633).
- Programs without key words (108) need a different oracle: their ARM words are not key words.
- The step time is derived from an assumed Master Clock scale (120 BPM); compare ratios (duty,
  step counts), not absolute times, until the scale is verified.

## Follow-ups

1. Decode the `DM(0x9512E..0x9515E)` and `DM(0x95331..)` word families into per-layer
   events and add them to the schema.
2. Name `DM(0xB2F25/6)` and `DM(0xB2FA1)` (constant, 95 per second).
3. Extend the window past 1.0 s so every arp step gets its off word.
4. Check that the gate programs found are exactly the programs whose arp mode is Gate
   (`arp-morph-flip-sweep.csv` has no mode column).
