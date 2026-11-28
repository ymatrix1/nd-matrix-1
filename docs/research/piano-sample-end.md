# Piano sample end: the firmware loops the stroke's loop region (issue #640)

- **Checked:** 2026-11-24
- **Sources (local, never committed):** OS v1.66 `target/fw_unpacked/bank0_CODE.bin`, SMP image `target/oracle/smp.bin`, factory backup `Nord Stage 4 Factory Restore RevK v1.66.ns4b`, `factory_programs/Program/Bank A/Astoria Grand.ns4p`; the run `tools/arm_dsp_live.py ... --piano-bind --key-ticks 80000 --log run` (SMP0.script, `smp_flash.npno`), replayed by `tools/smp_stream_end.py`.
- **Question:** what does the hardware do when a held note outlasts its sample? (1) does a factory `.npno` stroke set a loop, or does the SMP free the voice at the stream end; (2) what is the level law while it loops. Asked by the issue #640 reporter after D243 ("notes should gradually decay toward silence ... could the tail loop, each repetition quieter, until silent?").

Tags follow `.agents/rules/provenance-in-code.md`.

## 1. Short answer

| Question | Result | Tag |
|---|---|---|
| Does a factory stroke loop? | **Yes.** The ARM copies loop words from the `.npno` stroke record into the SMP voice record: loop start = `u32` at record byte `0x24`, loop length = `u32` at `0x28`, loop end `F-8` = start + length, stream end `F-9` = `total_frames` (`0x0E`). Key-up strokes carry zeros. | VERIFIED (2 of 2 strokes: 513 and 502 of Astoria Grand, s2); other sounds EMPIRICAL (s5) |
| What happens at `F-8`? | The counter wraps by `F-7` (smp-slew s3). The counter never reaches `F-9` while looping: **a held note is never freed by the stream end**, it repeats the stroke's loop region (about 33 % of the two verified strokes) until the key-up | VERIFIED (run, s3) |
| Level law in the loop | The record level `F15` keeps decaying by the rate word every slew update; once the counter is above `F-8 - F-7` the rate word is `F+6` (`d = 370`, 1.87 times the outside rate `F+7`, `d = 198` / `202`). In-loop decay is `exp(-d / 2^24 * 21,701 / s)` = **4.16 dB/s**, i.e. 6.5 dB per repetition of Astoria Grand stroke 513 | VERIFIED (run, s3) |
| Source of `F+6` | `d = 2 * -i16` of the `i16` at record byte `0x38` (`-185` on both strokes). Pitch scaling unknown (both notes were at the root) | ASSUMED (2 points) |

## 2. The stream words the ARM writes (`tools/smp_stream_end.py loop-words`)

SMP0.script pass 814 (note 60, velocity 100): `0x9391D .. 0x93924` are the record words `F-11 .. F-4` of voice 0 (`F = 0x93928`), the request blocks 12 words apart from `0x9251A` carry `F+6` (word +5) and `F+7` (word +6).

| Voice (stroke) | `F-9` end | `F-8` loop end | `F-7` loop length | loop start | `F+6` d | `F+7` d |
|---|---|---|---|---|---|---|
| 0 (513) | 162,532 | 162,505 | 54,149 | 108,356 | 370 | 198 |
| 1 (502) | 153,593 | 153,249 | 47,238 | 106,011 | 370 | 202 |

The same numbers sit in the `.npno` stroke record (`NpnoStroke`, `parse_loop`): `u32` at `0x24` = 108,356 and 106,011, `u32` at `0x28` = 54,149 and 47,238, `u32` at `0x0E` = 162,532 and 153,593. Test: `tools/tests/test_smp_stream_end.py::test_npno_record_fields_equal_the_voice_record_words`, `cbin::piano_sample::tests::stroke_loop_fields_follow_the_record`.

## 3. Observing the stream end (D235 gate)

The ring tap closes after ~2,300 frames (post stage gate, D235), so the held note cannot be heard past 24,000 words. The voice record itself is not gated: `tools/smp_stream_end.py replay` replays the note-on messages (nothing after pass 2,000: a held note) into the SMP chip of `sharc-live` for 260,000 passes (6 minutes) and reads `F-30` (counter), `F+8` (active), `F15`, `F17` directly. One pass is two audio frames (counter advance 1.596 = 2 x step 0.798), one slew update per pass.

Stroke 513 (voice 0), `F17` switches from `d = 198` to `d = 370` between pass 56,000 and 72,000 (counter 88,035 to 113,581, loop start 108,356):

| Pass | Counter `F-30` | Active | `F15` | `F17` |
|---|---|---|---|---|
| 8,000 | 11,398 | 1 | 0.4238 | `d = 198` |
| 56,000 | 88,035 | 1 | 0.2400 | 198 |
| 72,000 | 113,581 | 1 | 0.1920 | **370** |
| 104,000 | 110,523 | 1 | 0.0947 | 370 (counter wrapped at 162,505) |
| 136,000 | 161,614 | 1 | 0.0467 | 370 |
| 168,000 | 158,556 | 1 | 0.0230 | 370 |
| 248,000 | 123,837 | 1 | 0.0039 | 370 |

Stroke 502 (voice 1): same behaviour (counter 151,898 at pass 96,000, 130,206 at 112,000, active 1 to pass 256,000, `F15` 0.1121 to 0.0033, `F17` `d = 370`). The stream end `F-9` is never reached, nothing frees the voice (VERIFIED, 2 of 2 strokes, 260,000 passes = 12 s). Per-repetition ratio: `F15` 0.0787 to 0.0553 over 16,000 passes = 0.702 = `exp(-370 / 2^24 * 16,000)` (0.7027).

Real time of stroke 513: 162,505 / 1.596 passes x 2 / 43,402.34 Hz = 4.69 s to the loop end, a 1.56 s loop (key 60 of Astoria Grand).

## 4. What the engine does now (D255)

| Part | Engine | Evidence |
|---|---|---|
| Parse | `NpnoStroke::loop_start / loop_end / loop_decay` from the record | VERIFIED (s2) |
| Wrap | the read position wraps by the loop length at the loop end (`piano/sample_loop.rs::wrap`); no end, no tail fade for looped strokes (D243 stays for strokes without loop: SFZ, key-up) | VERIFIED (oracle edge case counter 130 gives 90 with `F-8 = 100`, `F-7 = 40`) |
| In-loop decay | the voice envelope is multiplied by `exp(-x * 21,701 / fs)` per frame above the loop start, `x = 2 * -i16 / 2^24`; the voice is freed below `1e-4` (-80 dB, about 19 s for `d = 370`) | VERIFIED rate, ASSUMED start at 1.0 |

On the opt-in `--piano-slew-fw` path the slew record takes `F+6` in the loop in place of `F+7` (`SlewVoice::set_rate`, test `piano::loop_tests::slew_path_looped_voice_decays_with_the_loop_rate`), and the voice allocator scores a slew voice by its slew gain (`decayed_slew_looped_voice_is_stolen_first`). SFZ regions loop only for `loop_mode=loop_continuous` / `loop_sustain`, `loop_end` inclusive, without decay (`piano/sfz.rs`, ASSUMED default `no_loop`).

Not modelled: the `F+7` decay (`d` about 200) of the hold before the loop (the default path plays the sample's own decay as recorded; the opt-in `--piano-slew-fw` chain applies it), predictor history continuity across the wrap (the engine reads the decoded PCM).

## 5. Other sounds: library survey (EMPIRICAL, not reproduced here)

The two verified strokes loop about 33 % of their length, which is where the first wording ("the last third") came from. A reviewer's survey of a local library of 2,196 strokes (review of PR #675, 2026-11-24) shows that is not typical:

| Quantity | Result over the surveyed strokes |
|---|---|
| Loop length / `total_frames` | 1.6 to 3.6 % for most strokes, median 2.7 %, maximum 22.7 % |
| Loop length | typically about 3.6k frames |
| `total_frames - loop_end` | 1k to 3k frames |
| Decay field `i16` at `0x38` | varies by sound |

Tag: `EMPIRICAL` (read from the files, never checked on the SMP). Only strokes 513 and 502 of Astoria Grand are `VERIFIED` on the SMP, which also shows this survey's loop words are the ones the voice record gets. The engine plays each stroke's own loop region, so nothing in the code depends on the third.

Method (repeatable, local-only; the library and the output stay out of git, D20 / D128):

1. Extract the `.npno` entries of the local factory backup (`Nord Stage 4 Factory Restore RevK v1.66.ns4b`, under `.reference/`) into a git-ignored folder, one file per sound.
2. For every file call `NpnoInstrument::parse(&bytes)` (`crates/nord-core/src/cbin/piano_sample.rs`) in a throwaway local program (a scratch `nord-tools` binary or test, not committed) and, for each stroke with `stroke_kind != 2` (key-up strokes carry no loop), read `total_frames`, `loop_start`, `loop_end` and `loop_decay`.
3. Tabulate `(loop_end - loop_start) / total_frames`, `total_frames - loop_end` and `loop_decay` per sound; report the distribution (median, maximum, 10th and 90th percentile) and the number of strokes.
4. Cross-check two strokes against `python tools/smp_stream_end.py loop-words` (513 and 502: 54,149 / 162,532 and 47,238 / 153,593).

## Implications

- The abrupt cut of #640 was the missing loop, not a library limit. D243 (fade) is superseded for `.npno` strokes by D255 (owner confirmation of the reversal pending); the fade stays as the tail rule of strokes without loop points.
- A looped note decays 4.2 dB/s: low notes (longer strokes, same rate) outlast high notes by their stroke length, as the reporter expects.
- OQ20 (end-of-sample behaviour) is answered for the sample end; the level at loop entry against the hardware stays open.

## Follow-ups

1. A hardware recording of a 10 s held note (known velocity) to confirm the level at loop entry, the `F+7` hold decay and the audible loop seam (predictor history after the wrap).
2. Verify the `i16` at `0x38` law (pitch scaling, other sounds): `tools/smp_stream_end.py loop-words` on a note 7 semitones above the root and on a second sound.
3. ~~Apply `F+6` inside `--piano-slew-fw`~~ done (D255, `SlewVoice::set_rate`).
