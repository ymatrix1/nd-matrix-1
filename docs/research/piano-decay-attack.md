# Piano decay (T60) and attack against the hardware clips

- **Checked:** 2026-11-15
- **Sources (local, never committed):** Part A demo video `_zksT69H30c`, factory backup `Nord Stage 4 Factory Restore RevK v1.66.ns4b` (piano library read through `ns4 render-patch` only), Transkun stream `docs/research/data/piano_events__zksT69H30c.json`; code `tools/emu_vs_hw.py --events-from transkun`, new `tools/piano_decay.py`; earlier work [piano-felt-upright-soft](piano-felt-upright-soft.md) (the "T60 3.2 s against 1.3 s" line), [piano-transcription](piano-transcription.md), [piano-receiver-scale](piano-receiver-scale.md) s7, [smp-slew](smp-slew.md). Derived numbers: [piano_decay_per_octave.json](data/piano_decay_per_octave.json).
- **Question:** Is there a decay (T60) and attack gap between the default piano engine (and the opt-in `--firmware-piano-chain --piano-slew-fw`) and the hardware, and which firmware parameter would close it?

Tags follow `.agents/rules/provenance-in-code.md`. All clips are replayed with the transcribed per-note velocities **and the sustain pedal** (D79), at HEAD `6e6b39d4`.

## 1. Method

| Item | Value |
|---|---|
| Per-octave T60 (new) | `tools/piano_decay.py`: for every note with no other note within +-1, +-12 or +-19 semitones in its fit window, the level of the fundamental (Hann DFT at `f0`, best of +-25 cents, 20 ms hops, 120 ms window, 250 ms below MIDI 48) is fitted in dB from 0.25 s after the onset to the next onset of the same pitch or 3 s; points within 35 dB of the peak; `T60 = -60 / slope`; median per octave. The pedal state is the same in the hardware and the emulator replay, so only the emulator-minus-hardware delta is read. EMPIRICAL heuristic (lossy, reverberant clip; 1 to 13 qualifying notes per octave, so single octaves are noisy) |
| Attack, hold, release, tail T60 | existing `emu_vs_hw.py` metrics (`attack_ms`, `emu_env`, `tail_t60_s`) |
| Single held notes | `ns4 render-patch` of one note (velocity 64, 14 s) for Felt Upright and Velvet Grand at MIDI 48 / 60 / 72, default and `--piano-slew-fw`; seconds from the peak 100 ms block to -10 / -20 / -30 / -40 dB |

## 2. Result: the default engine has no decay gap

Median emulator T60 over the hardware T60 of the same notes (octaves with at least 4 notes, T60 below the 30 s cap; 9 octave-groups over Velvet Grand, Felt Upright, White Grand, Pearl Upright): **1.00** (individual groups 0.59 to 2.86; the extremes are the 3 to 5 note groups below MIDI 48 and the White Grand bass).

| Clip | Octave | n | T60 hw s | default T60 emu s | chain + slew T60 emu s |
|---|---|---|---|---|---|
| Velvet Grand A:61 | 48-59 | 6 / 7 | 7.71 | 6.42 | 5.29 |
| Velvet Grand A:61 | 60-71 | 4 / 4 | 5.35 | 4.15 | 8.31 |
| Felt Upright A:13 | 48-59 | 10 / 10 | 9.01 | 9.00 | 28.06 |
| Felt Upright A:13 | 60-71 | 13 / 13 | 6.26 | 6.48 | 18.15 |
| White Grand A:21 | 48-59 | 4 / 4 | 8.84 | 5.24 | 8.03 |
| White Grand A:21 | 60-71 | 10 / 10 | 3.16 | 4.10 | 3.62 |
| Pearl Upright A:23 | 48-59 | 9 / 9 | 7.20 | 7.66 | 7.64 |
| Pearl Upright A:23 | 60-71 | 8 / 8 | 7.85 | 6.69 | 6.79 |

Tail T60 after the key release (`tail_t60_s`, 0.15 to 0.9 s after a release, hardware pedal included):

| Clip | hw s | default emu s |
|---|---|---|
| Felt Upright | 3.34 | 4.25 |
| Pearl Upright | 5.43 | 9.41 |
| White Grand | n/a (dense) | 8.9 |
| Velvet Grand | n/a (dense) | 0.92 |

The "T60 3.2 s hardware against 1.3 s emulator" of [piano-felt-upright-soft](piano-felt-upright-soft.md) s2 is **not reproduced**: it was read from the uniform-velocity, pedal-less replay. With the transcribed pedal the Felt Upright emulator tail is 4.25 s against 3.34 s (longer, not shorter). The decay of a default note is the library sample's own decay (no engine envelope on a stroke voice while held; the 80 / 180 ms release envelope applies only after key-up without pedal), and it equals the hardware within the scatter of the method. No decay parameter is fitted: there is no systematic gap to fit (D86).

The single held notes show the library itself: a note falls 40 dB in 3.7 to 5.8 s and is then silent (the sample ends; no loop is played by the default engine; since D250 the engine loops the last third of the stroke with the firmware in-loop decay, issue #640; D243 fade only for strokes without loop). Whether the hardware loops or holds the tail of the same sample past that point is not measurable from the clips (the clips' notes are released or pedalled long before; open, section 6).

## 3. Result: attack and level at this HEAD

Attack = median 10 to 90 % rise of the 2 ms envelope (ms); RMS = emulator minus hardware (dB).

| Clip | hw attack | default attack / RMS | chain attack / RMS | chain + slew attack / RMS |
|---|---|---|---|---|
| Velvet Grand | 64.4 | 54.2 / -2.6 | 54.2 / +0.1 | 69.7 / -4.0 |
| Felt Upright | 55.7 | 54.8 / -4.2 | 53.9 / -1.5 | 68.2 / -8.0 |
| White Grand | 58.0 | 45.9 / -4.8 | 45.9 / -2.2 | 47.3 / -3.7 |
| Pearl Upright | 58.1 | 48.3 / -4.7 | n/a | 49.1 / -4.9 |

- Default attack is 0 to 12 ms faster than the hardware (median about 10 ms, 17 %): a small gap in the direction of a sharper emulator, not the 2x of the earlier note.
- **The "about 2x slower attack" of chain + slew (121 / 99 ms in [piano-transcription](piano-transcription.md) s4) is not reproduced at this HEAD**: 69.7 / 68.2 ms on Velvet and Felt, 47 to 49 ms on White and Pearl (against 56 to 64 ms hardware). The slew path has no attack ramp (`F-2 = 0.25` from the first update, VERIFIED, [smp-slew](smp-slew.md) s6), so its attack is the stroke's own; the cause of the difference is unexplained (the earlier runs predate later slew and level changes; not bisected).
- Chain + slew is 1 to 4 dB below the default on Velvet, Felt and Pearl and 1 dB above it on White Grand: the level spread between sounds (-3.7 to -8.0) is the open absolute level of the slew gain, not a decay effect (OQ20).

## 4. What the firmware says about decay and attack

| Item | Value | Status |
|---|---|---|
| Hold decay of the SMP voice gain | `v' = v * F13 * F17`, `F17 = 1 - d / 2^24` per update at 43,402.34 / 2 Hz, `d = 118 .. 790` over velocity and key: time constant 2.1 to 5.0 s on Velvet Grand (Bright Grand `d = 224`, Wurlitzer 444, Amber Upright 284 at velocity 90) | VERIFIED per run ([smp-slew](smp-slew.md) s6); the law over sound is open, `rate_for` interpolates Velvet for every sound (ASSUMED) |
| Release of a note | factor `0.99958 * F17` per update: time constant 44 ms (60 dB in 0.30 s), identical for all velocities and sounds | VERIFIED (ARM note-off list) |
| Release sample record | `F17 = 0x3F7FF38E`, time constant 0.23 s, level `0.21875` at most; the engine keeps its own `amp * 0.30` and 1.15 s decay | VERIFIED messages, not modelled |
| Attack | no ramp: gain `0.21875 * level` from the first update; the attack is the sample's own onset | VERIFIED |
| Damper strokes | `NPNO_STROKE_DAMPER` strokes exist in the library but have no SMP record of their own | open (OQ20) |

The firmware hold decay is a gain on top of the sample, not a replacement of its decay. On the single notes of section 1 the slew path falls 20 dB after 0.7 to 1.7 s where the default needs 0.9 to 4.4 s (Felt C3 1.6 s against 4.2 s; Velvet C4 1.7 s against 4.4 s): the slew stacks the 2.1 to 5 s time constant on the sample decay, so the sample decays faster than the default on a single held note. On the dense clips the per-octave fit of the slew path is not usable for decay: its level is 4 to 8 dB under the default and the fundamental falls into the neighbours' leakage (T60 18 to 30 s on Felt Upright). There is no single-note hardware recording to say which is right.

## 5. Change

None to the engine. The evidence does not support a fit: the default decay equals the hardware (median ratio 1.00), the attack differs by about 10 ms, and the firmware parameters that shape decay are already in the opt-in slew path. A damper or release constant fitted to these clips would move the default away from the hardware in the cases where it now agrees (Felt Upright 9.01 / 9.00 s, Pearl Upright 7.20 / 7.66 s). The new `tools/piano_decay.py` and the committed per-octave numbers make the check repeatable (D86, D22).

## 6. Why chain + slew differs from the default (VERIFIED run, Velvet Grand C4, `ns4 render-patch`, velocity 30 / 64 / 100 / 127)

| Variant | peak of the first 300 ms (dB) | dB per velocity unit | vs default |
|---|---|---|---|
| default | -27.3 / -18.0 / -8.8 / -4.3 | 0.237 | 0 |
| `--piano-slew-fw` | -46.7 / -37.4 / -27.8 / -23.6 | 0.238 | -19.4 / -19.4 / -19.0 / -19.3 |
| `--firmware-piano-chain` | -24.7 / -15.4 / -6.1 / -1.7 | 0.237 | +2.6 |
| chain + slew | -44.1 / -34.8 / -25.2 / -21.0 | 0.238 | -16.8 |

1. **The slew follows the velocity law.** One note per velocity gives the same 0.238 dB per velocity unit as the default; the slope of about 0 in [piano-transcription](piano-transcription.md) s4 was an artefact of the dense clip (the dense-clip fits mix the paths, item 4).
2. **The slew is a constant 19.4 dB below the default at every velocity.** The firmware gain is `0.21875 * level` against the default's `level` (-13.2 dB, VERIFIED words); the remaining -6.2 dB is consistent with the muted extra voice (`F-2 = F3 = 0`, VERIFIED) that the default plays on top of the main voice (inference from the numbers, not isolated by a render without the extra voice). The absolute scale of the slew bus word is the open receiver convention (OQ20), so it is not a parameter to fit.
3. **Single-note attack is identical** (10 to 90 % rise: Felt 16 / 6 / 17 ms, Velvet 12 / 44 / 6 ms at MIDI 48 / 60 / 72, equal for default and slew). The 54 to 70 ms of the dense clips is therefore a mix effect (levels of overlapping notes, onset jitter of the transcriber, a 10 to 90 % window on the loudest note), not the voice envelope; only 2 to 3 notes per clip are isolated enough for a single-note attack, so the 10 ms default-versus-hardware attack difference cannot be resolved better than the method's scatter.
4. **With the damper pedal down the firmware voice path is bypassed** (`extra_voice.rs`: the damper-down strokes, `kind 1`, have no firmware voice list or SMP record in the decoded data, so the legacy path plays them): the pedal is down at the onset of 32 % (Velvet), 25 % (Felt), 77 % (White) and 62 % (Pearl) of the transcribed notes. A `--piano-slew-fw` clip replay is thus a mix of slew voices (pedal up) and legacy voices (pedal down), which explains why its level on the clips (-1.5 to -6.5 dB against the chain) is far above the -16.8 dB of a single note, and why the per-octave T60 of the slew render is not a clean measurement (section 2). Test: `extra_voice::tests::pedal_down_bypasses_the_slew_voice`.
5. **Level on isolated notes by pedal state** (median emulator minus hardware peak of the first 300 ms, notes with no onset in the next 0.35 s; Velvet 16 + 7, Felt 16 + 7, White 9 + 20, Pearl 17 + 19 notes pedal up + pedal down):

| Variant | Velvet up / down | Felt up / down | White up / down | Pearl up / down |
|---|---|---|---|---|
| default | +0.1 / -2.6 | -3.9 / -6.8 | -0.5 / -4.7 | -2.9 / -5.3 |
| chain | +2.8 / 0.0 | -1.2 / -4.1 | +2.2 / -2.1 | n/a |
| chain + slew | -2.3 / -1.8 | -8.9 / -6.9 | -3.6 / -2.7 | -2.5 / -2.8 |

For the default and the chain the pedal-down notes read 2.4 to 4.3 dB lower than the pedal-up notes, relative to the hardware: the emulator's damper-down strokes are quieter than the hardware's under the pedal (chain + slew shows no such step because its pedal-down notes are the legacy voices; EMPIRICAL, tails of earlier notes confound the peak; 7 to 20 notes per cell). No engine parameter is changed on this evidence (D22): the damper-down gain under the firmware path is ASSUMED, and a fit would mix the open bus scale into a per-stroke constant.

## Implications

- The T60 gap of OQ20 is closed as a measurement artefact; the open items are the slew path's absolute level and its per-sound rate table, and whether the hardware holds the sample tail after the last decoded sample.
- Quote T60 per octave from `tools/piano_decay.py`, not the whole-clip `tail_t60_s`, which fails on dense clips (Velvet: no value).

## Follow-ups

- A single-note hardware recording (known velocity, pedal up and down, 10 s) would measure the true T60, the release time constant (44 ms) and the sample end behaviour directly (the same request as [piano-felt-upright-soft](piano-felt-upright-soft.md)).
- Decode the firmware voice list of damper-down strokes (`kind 1`) so that `--piano-slew-fw` and the chain also cover notes struck under the pedal; then re-run the clip comparison, the largest remaining cause of the chain + slew gap.
- Run the ARM rate writer for Felt Upright, White Grand and Pearl Upright (`tools/arm_smp_slew_init.py rates`) to replace the Velvet-only `rate_for` table.
- Model the release-sample record (time constant 0.23 s) once the single-note recording exists.
