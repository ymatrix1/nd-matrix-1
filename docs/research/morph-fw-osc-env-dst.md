# Osc Env Amount DSP word by destination (OQ43)

**Checked:** 2026-11-21. **Sources (local, never committed):** OS v1.66 `bank0_CODE.bin` /
`bank0_SRAM.bin`, the factory backup `Nord Stage 4 Factory Restore RevK v1.66.ns4b`, harness
`tools/arm_osc_env_static.py` (new), `tools/arm_program_morph.py`, [morph-fw-vs-native](morph-fw-vs-native.md) s8.
**Question:** why does the DSP word of `Edt_Syn_kOscEnvAmt` equal `x / 60` on the 6 wheel programs
but a smooth other function on Cassette Pad AT, Favorite Video and Time Warp Saw (D206)?

## 1. Answer

The word depends on the layer's **Env To Pitch** bit (`kOscEnvDst`), not on the morph source.

| Fact | Evidence |
|---|---|
| All 3 A.T. programs have layer A `kOscEnvDst = 1`; the 7 other programs have it 0 on every layer with an amount | cells of 8 programs (decoder bit `sound_base + 123`) |
| A static dispatch of the amount (no morph, no source) gives the curve on Time Warp Saw and `x / 60` on Sine-Saw Plk Whl | [data/morph-fw-osc-env-dst.csv](data/morph-fw-osc-env-dst.csv), 121 steps each |
| Patching the Env To Pitch bit of the program file flips the law both ways (Time Warp Saw bit 0: `24 -> 24.0`; Sine-Saw bit 1: `24 -> 6.1875`) | same CSV, 4 runs of 121 steps |
| Both programs give identical words for the same bit and amount | `0xBF133336` at `-48` for Pitch, `0xBF4CCCCE` for Osc Ctrl |

## 2. The law

The setter `0xC01C61F0` (a parameter object with a selector word at `obj + 0xF0`, filled at program
recall by the descriptor copy ending at `0xC01CF5F8`) branches on the selector at `0xC01C6224`: `2`
stores the amount as is (`x / 60`), `1` runs it through `0xC01CD540`, a table taper:
`pos = 127 abs(u)`, `i = floor(pos)`, `y = T[i] + (T[i+1] - T[i]) (pos - i)`, word `= sign(u) y / 48`
(`u = x / 60`). `T` is the 128 entry table at `0xC02CA2C0` (`0..48`), already committed as
`LFO_PITCH_SEMITONES` (the same pitch amount taper as the LFO Pitch amount).

| Check | Result |
|---|---|
| Python model with float32 arithmetic vs 121 static steps of Time Warp Saw | max error 2.9e-5 steps |
| Rust `osc_env_dsp_word` vs 242 Pitch and 242 Osc Ctrl static rows | 484 of 484 within 1e-3 steps |
| Rust vs the 27 captured morph rows (`morph-fw-osc-env.csv`) | 21 Osc Ctrl + 6 Pitch of 27 |

Tests: `synth_morph_osc_env_tests.rs` (`the_dsp_word_law_matches_all_27_rows_by_destination`,
`the_dsp_word_law_matches_every_static_row_and_the_dst_flip`), `synth_osc_env_word.rs`,
`tools/tests/test_arm_osc_env_static.py`. The Env To Pitch file bit is bit 4595 of the `.ns4p`
(decoder `sound_base + 123`, with `sound_base + 71` at file bit 4543).

## 3. What stays open

* The word is the semitone depth over 48 if the DSP scales it by 48 (`ASSUMED`: the table is the
  pitch amount taper and `T[127] = 48`). The native engine maps `osc_env_amount` linearly to +/-12 st
  at full (`EMPIRICAL`, Audible Genius clip, about 1 st accuracy); the taper gives about 27.6 st at
  amount 48 and 48 st at 60. The scale of the word inside the ORG core 2 oscillator stage was not
  traced here; answered in [synth-osc-env-pitch](synth-osc-env-pitch.md) (D214: `48 * word * level` semitones).
* Layer B writes its word at `0xE01FE` (`EMPIRICAL`, one program); not explained here.

## Implications

The morph slot's knob value is verified for every source (dispatch 27 of 27). The DSP word is now
verified for every source and both destinations (27 of 27 plus 484 static rows). What the word means
for pitch is a separate engine question (OQ44), so the flag stays opt-in; the flip is the owner's
call (proposal in D213, no default changes).

## Follow-ups

OQ44 (DSP scale of word 12 under Env To Pitch, native depth law); the same selector for `kFltEnvAmt`
(word 13) was not checked; audible A/B of the 3 Pitch programs with `--synth-osc-env-morph`.
