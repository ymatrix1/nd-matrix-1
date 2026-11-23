# SMP chain tap: the `smp_cycles` outliers and Simple Tune (OQ41, D230)

- **Checked:** 2026-11-22
- **Sources (local, never committed):** the ranking CSV `docs/research/data/program-ranking.csv` (D216), `target/fw_unpacked/bank0_CODE.bin`, `bank0_SRAM.bin`, factory restore EEPROM, `target/oracle/smp.bin`, `target/PNO/core1_sram.bin`, factory backup `Nord Stage 4 Factory Restore RevK v1.66.ns4b` (`.npno` members read only), `factory_programs/Program/**` (read through `ns4 inspect --json`). Code: `tools/arm_smp_flash.py` (`tap_fundamental`), `tools/smp_pitch_audit.py` (new), `tools/arm_piano_bind.py`, `tools/arm_dsp_live.py`, `tools/program_rank_smp.py`. Earlier notes: [smp-voice-pitch](smp-voice-pitch.md), [program-ranking](program-ranking.md) s3.3a, [piano-bus-convention](piano-bus-convention.md).
- **Question (OQ41, follow-ups of D216):** are the nine `smp_cycles` outliers (x2 in 4 programs, x0.5 in 2, 0.0003 in 3) real octave shifts or a failing pitch estimate; which piano layer carries Simple Tune and can it be ranked?

Tags follow `.agents/rules/provenance-in-code.md`: **VERIFIED** = reproduced by a test or a command below, **EMPIRICAL** = measured in this harness, **ASSUMED** = reading.

## 1. Short answer

| Question | Result | Tag |
|---|---|---|
| The nine outliers | **Four are real octave shifts, five were a failing detector.** Momentum Whl, Shoreline Whl, Big Felt Pad Whl (piano layer A octave +1) read x2.01; J3 Split Special (octave -1) reads x0.50. French Harpsi (x2), Gradient Whl (x0.5), LoFi Piano, Clean Marimba AT, DistantGrand (0.0003) sound at the normal pitch | VERIFIED (`tools/smp_pitch_audit.py`: 152 rows after the fix, 148 `ok`, 4 `octave_shift`, 0 `mismatch`) |
| Cause of the five | `tap_fundamental` (harmonic sum, 4 harmonics, candidates 40..4000 bins of a 2^17 FFT) picked a sub-octave, a harmonic or its lowest candidate. Same taps, new detector (autocorrelation for the octave, harmonic sum inside it): 0.00602 to 0.00607 in all five | VERIFIED (`tools/tests/test_arm_smp_flash.py`; re-run of the 10 taps) |
| Does the tap level change? | No. `smp_peak_dbfs` of the nine is unchanged (the peak does not depend on the pitch estimate) | VERIFIED (same taps) |
| Simple Tune: which layer? | Piano **layer B** (layer A off, EP5 BrightTines XL, mono, octave +1). It plays on **piano core object 2** of the four in the pool, not core 0 | EMPIRICAL (4 cores tried) |
| Simple Tune: rank? | **Not possible with the ring tap.** On core 2 the key starts three voices (3 of 3 stroke descriptors, `law_ok`), their bus pointer is `0x9405A` and the bus holds a signal at the end of the run (14,015,760 = 0.0261 at `2^-29`), but the post stage ring (`0xB2000`, both halves) stays 0. The row keeps `piano_chain_no_audio` | EMPIRICAL |

## 2. The nine outliers

`smp_cycles` is the tap fundamental of key 60 in cycles per frame. The nominal value is 0.00603 (step law `0.80618 * 2^(tuning / 1200)`, D205); a layer octave of +1 / -1 doubles / halves the step. Octave = the `octave` field of the layer the loader plays (`ns4 inspect --json`, first enabled layer, else layer A).

| Program | Layer octave | `smp_cycles` before | Ratio | Verdict | `smp_cycles` now |
|---|---|---|---|---|---|
| Momentum Whl | A +1 | 0.01213 | x2.01 | real octave shift | 0.01213 |
| Shoreline Whl | A +1 | 0.01213 | x2.01 | real octave shift | 0.01213 |
| Big Felt Pad Whl | A +1 | 0.01213 | x2.01 | real octave shift | 0.01213 |
| J3 Split Special | A -1 | 0.00301 | x0.50 | real octave shift | 0.00301 |
| French Harpsi | A 0 | 0.01205 | x2.00 | detector: harmonic (strong even harmonics) | 0.00603 |
| Gradient Whl | A 0 | 0.00302 | x0.50 | detector: sub-octave | 0.00602 |
| LoFi Piano | no enable bit (layer A by the loader rule) | 0.00031 | x0.05 | detector: lowest candidate (floor) | 0.00602 |
| Clean Marimba AT | A 0 | 0.00031 | x0.05 | detector: lowest candidate (floor) | 0.00607 |
| DistantGrand | A 0 | 0.00031 | x0.05 | detector: lowest candidate (floor) | 0.00603 |

The 0.6 % excess of the octave rows over `2^octave * 0.00603` is the key 60 tuning of the sound ([smp-voice-pitch](smp-voice-pitch.md) s3). LoFi Piano and Gradient Whl bind the same sound (Mellow Upright Med 6.1); the old detector read 0.00031 on one and 0.00302 on the other, which says the estimate, not the sound, moved.

Why the old detector failed (VERIFIED on the re-run taps, spectra and autocorrelation compared):

| Failure | Mechanism |
|---|---|
| 0.00031 (3 programs) | the window starts at the loudest sample, frame 1,865 to 1,883 of a 5,968 frame tap, so it holds about 4,000 frames; the lowest candidate (bin 40 = 0.0003) collects slow decay and DC leakage at its first harmonic positions and wins |
| x0.5 (Gradient Whl) | the candidate at half the pitch counts every second harmonic of the true one plus the low frequency energy |
| x2 (French Harpsi) | harpsichord: the second and fourth harmonics are stronger than the fundamental, so `f` scores below `2f` |

Fix (`tap_fundamental`): the window starts at the onset (first frame above 5 % of the peak, 4,096 frames); the pitch is the **first** normalised autocorrelation maximum within 85 % of the best one over lags 33..1000 (cycles 0.03..0.001); the old harmonic sum then refines it inside +-3 % (autocorrelation alone reads 0.00606 where the harmonic sum reads 0.00602). A spectrum whose even harmonics hold more than about 85 % of the energy stays ambiguous by construction (it sounds an octave up); the test pins a harpsichord-like spectrum below that. The 10 taps (nine outliers plus Astoria Grand) land within 1.2 % of `2^octave * 0.00603` (EMPIRICAL: the thresholds were chosen on them).

`tools/smp_pitch_audit.py` is the standing check: it compares every `smp_cycles` with `2^octave` of the played layer. Before: 143 `ok`, 4 `octave_shift`, 5 `mismatch` (the five detector rows). After the CSV update: 148 `ok`, 4 `octave_shift`, 0 `mismatch`. The other 143 rows keep their old values (they are within 0.6 % of the nominal and in the right octave, so the old detector was right on them).

## 3. Simple Tune (piano layer B only)

The program stores layer A off and layer B on (EP5 BrightTines XL 6.1, mono, octave +1, level 0.77). `arm_piano_bind` bound the sound to core 0, slot 0, as for every census program; the key sent 381 SMP messages and no stroke descriptor.

| Experiment (key 60, 3,000 ticks, `--fifo-pace 1`) | Key phase SMP messages | Stroke descriptors | Tap |
|---|---|---|---|
| layer B sound on core 0, slot 0 (census) | 381 | 0 | silent |
| same, plus the binder `0xC00A3988(core, 1, P, 0)` (slot 1), with and without slot 0 | 381 | 0 | silent |
| layer **A** sound (Felt Upright) on core 0 | 381 | 0 | silent |
| layer B sound on core 1 | 381 | 0 | silent |
| layer B sound on **core 2** | **489** | **3** (strokes 357, 355, 364; `law_ok` 3 of 3, ring equals the file) | silent |
| layer B sound on core 3 | 381 | 0 | silent |
| Astoria Grand (layer A) on core 2 | 381 | 0 | silent |

Reading (EMPIRICAL): the program state, not the sound, decides which core starts a voice. With layer A off and B on, the key path acts on the core object two core strides (`0x176C0` = 95,936 bytes) after core 0; layer A programs act on core 0. The four candidates come from `find_cores` (pool pointer at `+0x14`). Which of core 1 and core 3 belongs to which scene or layer pair is not identified. The slot binder does not matter (the key path reads slot 0 of the core).

**Where the audio goes.** The voice records differ in the bus pointer (`rec[-18]`, [smp-voice](smp-voice.md) s2): layer A voices write `0x0009404E` (record at `0x93E92`), the Simple Tune voices `0x0009405A` (record at `0x93CAC`), 12 words (two frames) after it. At the end of the run `DM(0x9405A)` and `DM(0x9405B)` both hold 14,015,760 (0.0261 at `2^-29`, -31.7 dBFS for that one sample), so the voice decodes and the bus gets a signal. The post stage ring `0xB2000..0xB202B` (both halves) holds only the gate fade (order 1e-5); the tap of the other half (`H 1`) is silent too. So the ring tap of D200 carries the layer A bus `0x9404E` only.

Consequence for the ranking: the 33 programs with both piano layers enabled have a tap of layer A only; Simple Tune has no tap. A bus tap (`sharc-live --smp` reading `DM(0x9405A)` per interrupt) is the smallest next step; it would also give layer B of the 33 two-layer programs.

## 4. Tool changes

| Change | File | Test |
|---|---|---|
| `tap_fundamental`: autocorrelation octave, harmonic-sum refinement, onset window | `tools/arm_smp_flash.py` | 3 new cases in `tools/tests/test_arm_smp_flash.py` (even-harmonic tone, drift plus late onset, octave steps) |
| `core_of`, `LAYER_CORE`, `CORE_STRIDE`; `bind_piano(layer=)`; `bind_program_piano` passes the active layer | `tools/arm_piano_bind.py`, `tools/arm_dsp_live.py` | `test_core_of_follows_the_layer_core_stride` |
| `smp_pitch_audit`: ratio of `smp_cycles` to `2^octave` of the played layer | `tools/smp_pitch_audit.py` | `tools/tests/test_smp_pitch_audit.py` (3) |
| CSV: `smp_cycles` of French Harpsi, Gradient Whl, LoFi Piano, Clean Marimba AT, DistantGrand (5 cells) | `docs/research/data/program-ranking.csv` | `tools/program_rank_smp.py` merge, audit above |

Layer A programs are unaffected by the bind change (`LAYER_CORE["layer_a"]` = 0); Simple Tune is the only census program with layer B alone (the five programs with no enable bit play layer A).

## Reproduce

```bash
M=target; B=".reference/Nord Stage 4 Factory Restore RevK v1.66.ns4b"
cargo build --release -p nord-sharc --bin sharc-live
# one program, SMP chip, tap in LOG/smp_tap.f32 (about 3 min); the command line is the one
# `program_rank_chain.live_command` builds for chips PNO1 + SMP0 with a log directory
uv run --with unicorn --with capstone python tools/arm_dsp_live.py $M/fw_unpacked/bank0_CODE.bin \
    $M/fw_unpacked/bank0_SRAM.bin --eeprom $M/oled/eeprom.bin --program "factory_programs/Program/Bank B/Gradient Whl.ns4p" \
    --live $M/release/sharc-live --poll-advance 64 --ns4b "$B" --out run.json --chip "PNO:1=pno=$M/PNO/core1_sram.bin" \
    --chip "SMP:0=smp=$M/oracle/smp.bin" --piano-bind "$B" --log LOG --key-ticks 3000
# tap pitch of the file, then the CSV merge (chain.jsonl: the `census.smp` records) and the audit
python -c "import sys; sys.path.insert(0, 'tools'); import arm_smp_flash as sf; print(sf.tap_fundamental(open('LOG/smp_tap.f32', 'rb').read()))"
python tools/program_rank_smp.py --chain chain.jsonl --csv docs/research/data/program-ranking.csv
python tools/smp_pitch_audit.py --csv docs/research/data/program-ranking.csv --ns4 target/release/ns4.exe --programs factory_programs
```

The per-program tap analysis of this note ran `arm_dsp_live.py` with `--log DIR` (tap file `smp_tap.f32`) on the nine programs and Simple Tune.

## Implications

- `smp_cycles` is now a usable per-program pitch check; the four octave rows are the programs' own layer octave and need no tool change.
- A layer B only program binds to core 2 in the harness; programs with both layers bind layer A only.
- The ring tap does not carry layer B: ranking layer B (Simple Tune, and the second layer of 33 programs) needs a tap of the bus words.

## Follow-ups

1. Bus tap in `sharc-live --smp` (`DM(0x9405A..)` per interrupt), then rank Simple Tune and read layer B of the 33 two-layer programs.
2. Identify cores 1 and 3 (scene II, or the second section).
3. Run the five re-detected programs in the next full census so `smp_peak_dbfs` and `smp_cycles` come from one code version (the peaks do not change).
