# Organ presence gap re-run on the merged note stream: B3Wheels still loses the 1.5 to 8 kHz bands on B3 Chords n Solo

- **Checked:** 2026-11-22
- **Sources (local, never committed):** Part A demo video `_zksT69H30c` (segments `organ_b3` = `B3 Chords n Solo` at 238 s, `fx_reverb` = `B3 Old Forest` at 2078 s), factory `.ns4b` v1.66, `tools/emu_vs_hw.py` and `tools/organ_gap.py` on this branch (release `ns4`), the committed Transkun stream `piano_events__zksT69H30c.json`, spec `data/emu_vs_hw_organ_presence_rerun.json`, results `data/emu_vs_hw_organ_presence_rerun_results.json`. Builds on [organ-presence-gap](organ-presence-gap.md) and [b3-leslie-dark](b3-leslie-dark.md) (D220, `--merge-gap`). No audio is committed.
- **Question:** [organ-presence-gap](organ-presence-gap.md) found `B3Wheels` 7 to 20 dB under the clip at 1.5 to 8 kHz on the video-confirmed notes, and D220 showed that note set was the wrong one (brilliance -12.8 to +11.6 dB across key zones). With the merged Transkun stream, does the evidence now support flipping the B3 default to `B3Wheels` (D22)?

Tags follow `.agents/rules/provenance-in-code.md`. Tables: emulator minus hardware, dB re RMS. Score: lower is better. `bw` = `--tone-source b3wheels`; `bw+r+l` = `--tone-source b3wheels --firmware-routing --b3-tail on --organ-level firmware` (the "best opt-in path" of the earlier note); `rot` adds `--rotary-drive scaled`; `lfw` adds `--leslie firmware`. Streams: "confirmed" = `--confirmed-only` (101 notes, the stream of the old verdict, **before**); "raw" = `--events-from transkun` (525 notes); "merged" = `--events-from transkun --merge-gap 0.3` (96 notes, **after**).

## 1. Answer in short

| Item | Result | Tag |
|---|---|---|
| Does the merged stream close the `B3Wheels` gap? | **Partly, not enough.** `bw+r+l` presence / brilliance on B3 Chords: -18.9 / -13.0 (confirmed), -1.5 / +4.5 (raw), **-10.9 / -9.0 (merged)**. Legacy default on the same streams: -10.0 / -12.8, +2.7 / +10.0, **-3.0 / +1.3**. The stream moves both paths by 10 to 17 dB, but the distance B3Wheels minus Legacy stays: presence -8.9, -4.2, -7.9 dB; brilliance -0.2, -5.5, -10.3 dB | VERIFIED (run, s2) |
| Score on the merged stream | Legacy **4.04**, `bw+r+l` **7.22**, `bw` 5.99, `bw+r+l+lfw` 4.12 (Legacy is better) | VERIFIED (run, s2) |
| Lines | Peakiness 1.5 to 3 kHz on the merged stream: hardware 22.0 dB, Legacy 14.8, `bw+r+l` **11.8** (noise-like). 3 to 6 kHz: 20.3 / 15.5 / 11.1. The partial lines the hardware has are still not in the B3Wheels render with the right-hand notes replayed, so "the replay is left-hand" does not explain the `B3Wheels` deficit any more | VERIFIED (run, s3) |
| Second B3 clip (Old Forest) | Mixed. Score `bw+r+l` against Legacy: confirmed 4.88 / 3.92 (Legacy better), raw 5.31 / 5.98 (B3Wheels better), merged 4.38 / 5.52 (B3Wheels better; presence +5.2 / +6.7, brilliance +5.4 / +8.5, RMS +0.3 / +4.3). The two clips disagree | VERIFIED (run, s4) |
| Flip `B3Wheels` default? | **No: the evidence does not support it.** Owner-ready proposal and flip criterion in s5 | proposal |
| Engine, default, `render_hash` | unchanged; no Rust touched | VERIFIED |

## 2. Per-band table, B3 Chords n Solo (`organ_b3`, 25 s)

Bands: sub 20..120 / low 120..400 / mid 400..1.5k / pres 1.5..3k / brill 3..8k / air 8..20k Hz.

| Stream | Variant | sub | low | mid | pres | brill | air | RMS | score |
|---|---|---|---|---|---|---|---|---|---|
| confirmed (101, before) | default (Legacy) | -0.7 | -0.2 | +3.5 | -10.0 | -12.8 | +17.4 | +0.9 | 5.45 |
| | bw | +1.3 | +1.2 | +2.1 | -12.1 | -4.1 | +27.8 | -9.4 | 4.14 |
| | bw+r+l | +1.4 | +1.4 | +2.2 | -18.9 | -13.0 | +18.9 | -2.8 | 7.36 |
| | bw+r+l+rot | +1.4 | +1.4 | +2.2 | -18.9 | -12.9 | +19.0 | -6.8 | 7.34 |
| | bw+r+l+lfw | +1.1 | +0.8 | +3.8 | -6.5 | -2.7 | +10.7 | -11.0 | 2.98 |
| raw (525) | default (Legacy) | +7.0 | -3.9 | +4.2 | +2.7 | +10.0 | +33.8 | +0.4 | 5.56 |
| | bw | +11.2 | -2.1 | +2.5 | +5.0 | +10.9 | +44.7 | -10.6 | 6.35 |
| | bw+r+l | +12.2 | -1.2 | +3.4 | -1.5 | +4.5 | +36.8 | -4.8 | 4.55 |
| | bw+r+l+rot | +12.2 | -1.2 | +3.4 | -1.4 | +4.5 | +36.8 | -8.9 | 4.53 |
| | bw+r+l+lfw | +11.0 | -3.3 | +3.6 | +5.1 | +6.9 | +22.0 | -11.0 | 5.98 |
| merged (96, after) | default (Legacy) | +10.4 | -1.4 | +4.1 | -3.0 | +1.3 | +15.6 | +3.5 | 4.04 |
| | bw | +12.8 | -0.0 | +3.1 | -9.2 | -4.9 | +25.2 | -7.3 | 5.99 |
| | bw+r+l | +12.9 | +0.1 | +3.2 | -10.9 | -9.0 | +16.4 | -0.7 | 7.22 |
| | bw+r+l+rot | +12.9 | +0.1 | +3.2 | -10.9 | -9.1 | +16.4 | -4.7 | 7.22 |
| | bw+r+l+lfw | +12.9 | -2.0 | +4.5 | -1.1 | -0.2 | +2.2 | -8.2 | 4.12 |

Before to after (confirmed to merged). `bw+r+l`: presence -18.9 to -10.9 (+8.0 dB), brilliance -13.0 to -9.0 (+4.0), RMS -2.8 to -0.7, score 7.36 to 7.22. Legacy: presence -10.0 to -3.0 (+7.0), brilliance -12.8 to +1.3 (+14.1), RMS +0.9 to +3.5, score 5.45 to 4.04. Legacy gains more from the right note set, so the brilliance distance between the two grows (-0.2 to -10.3 dB).

The sub band is +10 to +13 dB for every path on the raw and merged streams (replayed low notes; the open sub item of D220). It is the same for Legacy and B3Wheels, so it does not separate them.

## 3. Long-term spectrum and tonal peakiness, merged stream (`tools/organ_gap.py`)

dB re clip total:

| band Hz | hw | Legacy | bw+r+l | bw+r+l+lfw |
|---|---|---|---|---|
| 800-1000 | -13.5 | -8.5 | -12.1 | -10.4 |
| 1000-1500 | -11.7 | -17.5 | -23.4 | -19.4 |
| 1500-2000 | -14.8 | -22.1 | -27.7 | -21.6 |
| 2000-3000 | -18.0 | -24.1 | -29.7 | -18.6 |
| 3000-4000 | -24.5 | -28.6 | -35.1 | -26.3 |
| 4000-6000 | -38.9 | -29.2 | -38.3 | -41.5 |

| peakiness dB | hw | Legacy | bw+r+l | bw+r+l+lfw |
|---|---|---|---|---|
| 100-400 | 27.5 | 12.3 | 12.6 | 15.3 |
| 400-800 | 22.9 | 13.4 | 15.3 | 19.3 |
| 800-1500 | 24.3 | 21.3 | 23.9 | 21.5 |
| 1500-3000 | 22.0 | 14.8 | 11.8 | 11.0 |
| 3000-6000 | 20.3 | 15.5 | 11.1 | 21.3 |

On the merged stream the 0.8 to 1 kHz hole of the earlier note (hardware -13.5, emulator -25.6) is gone (-8.5 / -12.1): that part of the earlier gap was the replay. The 1 to 3 kHz deficit remains for both paths and is 5.6 to 5.9 dB larger for `B3Wheels` than for Legacy. Only the firmware Leslie (not installed in the firmware, D190) raises the 3 to 6 kHz lines, at RMS -8.2 dB.

## 4. B3 Old Forest (`fx_reverb`, 20 s)

| Stream | Variant | sub | low | mid | pres | brill | air | RMS | score |
|---|---|---|---|---|---|---|---|---|---|
| confirmed | default | -0.6 | -0.8 | -3.1 | +7.7 | -7.4 | +8.5 | +3.6 | 3.92 |
| | bw+r+l | -1.9 | +0.6 | -4.6 | +3.6 | -13.7 | +12.3 | -0.2 | 4.88 |
| raw | default | +2.2 | -6.5 | -4.5 | +6.7 | +9.9 | +11.0 | +2.1 | 5.98 |
| | bw+r+l | +6.1 | -3.0 | -4.3 | +6.6 | +6.5 | +12.4 | -1.4 | 5.31 |
| merged | default | +1.2 | -6.5 | -4.6 | +6.7 | +8.5 | +7.0 | +4.3 | 5.52 |
| | bw+r+l | +5.6 | -2.4 | -3.4 | +5.2 | +5.4 | +6.7 | +0.3 | 4.38 |

`B3Wheels` wins on raw and merged (closer presence, brilliance and RMS) and loses on confirmed. This is a different program (spring reverb 0.62, hardware T60 42 s on the confirmed stream); its ranking does not transfer to the chords clip.

## 5. Owner-ready proposal on D22

- **Recommendation: do not flip `B3Wheels` to the B3 default.**
- Against: on the stream D220 shows to be right (merged), B3 Chords scores 4.04 for Legacy against 7.22 for `bw+r+l`, with presence 7.9 dB and brilliance 10.3 dB worse and the 1 to 3 kHz lines absent (peakiness 11.8 against 22.0 dB). Old Forest favours `B3Wheels` (4.38 against 5.52), so the clips disagree. The D22 bar (algorithm, mapping and end-to-end match all VERIFIED) is not met: `B3Wheels` is still not within 1 dB on every chord line (OQ16).
- For a later flip: with the stream fixed, `B3Wheels` has the better RMS (-0.7 against +3.5 dB) and low band (+0.1 against -1.4 dB), and Old Forest prefers it.
- **Flip criterion (proposed):** `bw+r+l` on the merged stream within 3 dB at presence and brilliance, and a score no worse than Legacy on both B3 clips. Missing: the 1 to 6 kHz lines. Candidates, in order: the firmware Leslie install (D190; lifts presence / brilliance by +9.8 / +8.8 dB but RMS reads -8.2), the tail's phase and the multi-write `A` words (OQ16), a second B3 recording with a known note set.
- A flip changes `render_hash` for every B3 program and needs the D22 program sweep; not done here. The owner decides.

## 6. What this changes elsewhere

- [organ-presence-gap](organ-presence-gap.md) s1 (B3): the 0.8 to 1.5 kHz hole and the "-19.9 dB" presence of `b3wheels` were partly the note set (merged: presence -10.9 dB, the 0.8 to 1 kHz hole closed). The 1.5 to 3 kHz line deficit is not.
- The Vox clip was not re-run here (its open piece is the amp residual); follow-up 1.

## Implications

- Keep `B3Wheels` opt-in; report B3 verdicts on the merged stream (`--events-from transkun --merge-gap 0.3`) next to the confirmed one.
- The next B3 lever is the lines at 1 to 6 kHz of the right-hand notes, not a note-set or stage fit.

## Follow-ups

1. Re-run the Vox clip (`organ_vox`, `vox` + level) with the merged stream and compare with the 7.5 / 9.7 dB residual.
2. Apply `note_offsets.audio_offsets` after the merge (second flag) and re-run s2.
3. A B3 clip with a known note set (a scripted chord set recorded on the hardware) to separate the tail phase from the replay.

Reproduce:

```bash
cargo build --release -p nord-core
for X in "--confirmed-only" "--events-from transkun" "--events-from transkun --merge-gap 0.3"; do
  uv run python tools/emu_vs_hw.py --spec docs/research/data/emu_vs_hw_organ_presence_rerun.json --video <Part A .webm> --backup <factory .ns4b> --workdir <scratch> $X
done
python tools/organ_gap.py <scratch>/organ_b3 emu_default.wav "emu_bw+routing+level.wav"   # merged workdir
```
