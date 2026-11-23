# B3 Leslie "too dark" (-12.8 dB at 3..8 kHz) is the replayed note set, not a stage

- **Checked:** 2026-11-22
- **Sources (local, never committed):** the Part A demo video and the factory backup `.ns4b` under `.reference/`; `tools/emu_vs_hw.py` on the `organ_b3` clip (`B3 Chords n Solo`, 238 s, 25 s) with `docs/research/data/emu_vs_hw_b3_leslie_dark.json` (results next to it as `_results.json`); the committed Transkun stream `piano_events__zksT69H30c.json`; prior notes [treble-excess-by-engine](treble-excess-by-engine.md) (D215), [organ-presence-gap](organ-presence-gap.md), [b3-rotary](b3-rotary.md), [b3-leslie-install-hunt](b3-leslie-install-hunt.md) (D190). No firmware byte, sample or audio is printed or committed.
- **Question:** audio-remaining row 9 and D215 read the B3 clip 12.8 dB below the hardware in 3..8 kHz. Which stage (tonewheel, drawbar, key click, drive, Leslie crossover / horn / drum, cabinet) is responsible, and is there a fix?

Tags follow `.agents/rules/provenance-in-code.md`.

## 1. Short answer

| Item | Result | Tag |
|---|---|---|
| Responsible stage | **None.** The same engine chain reads brilliance (3..8 kHz, emulator minus hardware, dB re RMS) from **-12.8 to +11.6 dB** by changing only which notes are replayed: video-confirmed notes (101, MIDI 36 to 71) -12.8; every detected note (285) -7.6; detected notes MIDI 36..59 -12.2, 60..71 -8.4, 72 and up **+11.6**; the 525 Transkun notes +10.0; Transkun with same-key re-triggers merged at 0.3 s (the player's 96 notes) **+1.3**. Presence reads -3.0 on that stream | VERIFIED (run, s2) |
| Why | The clip is a right-hand comping and solo; `--confirmed-only` keeps the left-hand octaves 3 and 4. A tonewheel organ puts its 3..8 kHz energy in the upper partials of high keys, so a low note set has no brilliance to compare with a clip that has it. The D215 value is a property of the replay, as the piano velocity was for piano_dry | VERIFIED (zones, s2) |
| Rotary (engine Leslie) | Not the darkening stage. On the full note set it adds treble share (rotary off -10.5 against +1.3 on, re RMS); on the confirmed set it costs 5.5 dB. The sign follows how much high content there is to spread, which a stage error would not do | VERIFIED (run, s3) |
| Firmware Leslie (`--leslie firmware`) | Overshoots on the right note set (presence +4.8, brilliance +5.7, score 5.80 against 4.04). Not the answer to row 9; stays opt-in, no change | VERIFIED (run, s3) |
| Fix | Tool only, opt-in: `emu_vs_hw.py --merge-gap SECONDS` (with `--events-from`) joins same-key re-triggers of the stream (`note_offsets.merge_retriggers`). **No engine, default or `render_hash` change.** No stage is added: a presence or brilliance boost would be fitted to a wrong note set ([organ-presence-gap](organ-presence-gap.md) s4 warned of this) | VERIFIED (run, test `test_segment_events_merge_gap_joins_retriggers`) |
| Remaining on B3 | RMS and the sub share (+10 dB with the rotary on) and the firmware Leslie install (D190) stay open. Presence -3.0 is at the 3 dB bar | open |

## 2. One chain, seven note sets

`tools/emu_vs_hw.py`, variant `default` (current `main`, engine rotary), emulator minus hardware, dB re RMS. Data: `data/emu_vs_hw_b3_leslie_dark_results.json`.

| Replayed notes | n | sub | low | mid | pres | brill | RMS | score |
|---|---|---|---|---|---|---|---|---|
| confirmed (`--confirmed-only`) | 101 | -0.7 | -0.2 | +3.5 | -10.0 | **-12.8** | +0.9 | 5.45 |
| all detected | 285 | +9.6 | -0.5 | +3.3 | -9.0 | -7.6 | +6.0 | 5.98 |
| detected, MIDI 36..59 | | +4.2 | -0.3 | +3.2 | -16.9 | -12.2 | | |
| detected, MIDI 60..71 | | +14.4 | -0.4 | +2.1 | -4.0 | -8.4 | | |
| detected, MIDI 72 and up | | -21.1 | -4.5 | +3.1 | +8.9 | **+11.6** | -11.9 | 9.86 |
| Transkun as is | 525 | +7.0 | -3.9 | +4.2 | +2.7 | +10.0 | +0.4 | 5.56 |
| Transkun, `--merge-gap 0.3` | 96 | +10.4 | -1.4 | +4.1 | **-3.0** | **+1.3** | | 4.04 |

Brilliance is a monotone function of the highest keys replayed (-12.2, -8.4, +11.6 over the three zones, 24 dB end to end), and the merged stream lands inside 3 dB in the treble. The 525-note stream is too bright because the Transkun re-triggers (a Leslie amplitude modulation reads as repeated notes) stack extra attacks; the merge removes them.

## 3. Stage ablations on the wrong and the right note set

Same spec, columns pres / brill. `rotary-scaled` = `--rotary-drive scaled`; `leslie-fw` = `--leslie firmware`; `fw-all` = `--firmware-all --firmware-routing`.

| Variant | confirmed | merged Transkun (96) |
|---|---|---|
| default | -10.0 / -12.8 | -3.0 / +1.3 |
| rotary off (`organ.rotary_enabled`) | -12.2 / -7.3 | -15.0 / -10.5 |
| rotary-scaled | -10.1 / -12.8 | -3.0 / +1.3 |
| leslie-fw | -2.4 / -3.2 | +4.8 / +5.7 |
| fw-all | -10.7 / -12.2 | -4.3 / +2.1 |

`leslie-fw` is 5 to 8 dB brighter on either set and models a stage whose feed the firmware install does not show (D171, D172, D190), so it is not evidence for the hardware level.

## 4. What this closes and what it leaves

- Row 9, brilliance: **closed by evidence**. -12.8 dB was the replay; the merged stream reads +1.3 dB. Presence is -3.0 on the same stream, at the bar.
- Row 23 / D215 "B3 -12.8 dB": superseded by this note (D215 text is not edited).
- Still open: the firmware Leslie install (D190), the clip RMS and sub share, and a second B3 clip with a known note set (no controlled hardware B3 recording exists in `.reference/`; the DSoundman clip is a drawbar demo without Leslie).

## Implications

- No B3 stage gets a fix or a default flip. The presence-gap hunts that used the low-octave replay (D22 bar) need re-reading with `--merge-gap 0.3`.
- Any keyboard clip comparison is only as good as its note stream: report the stream next to the number.

## Follow-ups

1. Re-run [organ-presence-gap](organ-presence-gap.md) s2 (B3 and Vox, `b3wheels` + routing + level) with `--events-from transkun --merge-gap 0.3`; it may reopen the `B3Wheels` default flip (D22), owner decision.
2. Same re-run for the Vox clip and B3 Old Forest.
3. Optional: apply `note_offsets.audio_offsets` after the merge as a second flag.

Reproduce:

```bash
cargo build --release -p nord-core
uv run python tools/emu_vs_hw.py --spec docs/research/data/emu_vs_hw_b3_leslie_dark.json --video <Part A .webm> --backup <factory .ns4b> --workdir <scratch> --confirmed-only
uv run python tools/emu_vs_hw.py --spec docs/research/data/emu_vs_hw_b3_leslie_dark.json --video <Part A .webm> --backup <factory .ns4b> --workdir <scratch> --note-range 72 108
uv run python tools/emu_vs_hw.py --spec docs/research/data/emu_vs_hw_b3_leslie_dark.json --video <Part A .webm> --backup <factory .ns4b> --workdir <scratch> --events-from transkun --merge-gap 0.3
```
