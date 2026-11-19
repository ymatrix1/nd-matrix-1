# FX Mod 2 CHOR against a hardware clip: oscilloscope edge separation

- **Checked:** 2026-11-14
- **Sources (local, never committed):** `.reference/chorus-nord-vs-ultimate-pianos2/` (video by Junior Porciuncula, "Nord Stage 4 Chorus against the Chorus of Kontakt Ultimate Pianos 2": `clip.mp4` AV1 1080x1920 30 fps + AAC 44.1 kHz, 41.8 s; `clip.srt`; `L.wav` = left channel = Nord Stage 4 Mod 2 Effect B Chorus on a piano, `R.wav` = right channel = the plugin chorus, both mono). Tools: [tools/fx_chorus_clip.py](../../tools/fx_chorus_clip.py), `crates/nord-core/examples/fx_chorus_probe.rs`. Derived data: [data/fx-chorus-hw-scope.csv](data/fx-chorus-hw-scope.csv), [data/fx-chorus-probe-grid.csv](data/fx-chorus-probe-grid.csv).
- **Question:** which Nord CHOR type (`kFx2Type` 4 or 5) matches the hardware, and are the firmware chorus (`FirmwareFx::mod2`) and the `Amt` / `Rate` laws closer to it than the engine chorus (D22 bar: no side worse than about 3 dB)?

Tags follow `.agents/rules/provenance-in-code.md`.

## 1. Short answer

| Quantity | Hardware clip | Firmware `fw4` (type 4) | Engine chorus | Tag |
|---|---|---|---|---|
| LFO rate | 0.336 Hz (47 frames, spectral peak); 4 maxima at 24.5, 27.5, 30.5, 33.0 s | 0.343 Hz at rate word `v = 30` (`12800 + 4096 v`) | `mod2_rate_hz` as set | EMPIRICAL: rate matches the law for `v = 30` |
| Delay of the copy (5th to 95th percentile) | 0.62 to 1.78 ms, mean 1.17 ms | 0.87 to 3.28 ms, mean 2.02 ms (amount 8..64) | 4.5 to 13.5 ms at amount 1 (9 ms centre) | EMPIRICAL |
| Swing / centre | 0.48 | 0.58 | 0.50 | EMPIRICAL |
| Second edge over first (wet gain) | 0.86 (median, 54 rows) | 0.82 at amount 64, 0.88 at 90 (about 80 for 0.86) | 1.0 at amount 127 | EMPIRICAL |
| Nord trace against plugin trace | within 0.1 ms on all 7 overlapping frames | n/a | n/a | EMPIRICAL |

Verdict: the rate agrees with CHOR type 4 and the wet gain with an amount near 80; the **absolute delay is 0.55 times the firmware's** (and 0.13 times the engine's) if the displayed sawtooth is the 87.31 Hz note. The clip does not show that, so **CHOR stays opt-in** (D60). Type 5 cannot be told from the clip: its output is not a clean delayed copy of an edge (s4), the scope shows one.

## 2. What the clip contains

| Time (s) | Content |
|---|---|
| 0 to 6 | Nord panel, desk, DAW; narration |
| 6 to 22 | Logic Pro tracks with the Kontakt panel (Effect B = Chorus, Rate and Amount knobs, not readable); L / R playing chords (dominant peaks 100 to 765 Hz) |
| 22 to 38 | one sustained note, fundamental 87.31 Hz (F2; harmonic comb fit 86.5 to 88.2 Hz, `tools/fx_chorus_clip.py` `F0_HZ`) with a DAW oscilloscope in frame from 22.5 s to about 35 s: two traces (white upper, yellow lower) |
| 38 to 41.8 | DAW overview |

* `L.wav` and `R.wav` are mono (identical left and right samples), levels within 0.3 dB from 22 to 38 s (up to 3 dB earlier), envelope correlation 0.92, harmonics 1 to 4 of the F2 note equal to three digits, harmonics 5 and up differ (`0.008` against `0.014`, `0.004` against `0.009`).
* **No dry / wet A/B inside the clip**: both channels are wet for the whole time. Chorus on in the left channel is inferred from the narration and the scope, not from a dry reference.
* `tools/video_ref.py` note detection was **not** used: the keyboard and OLED are visible only in the first two seconds, so the notes come from the audio (comb fit above).
* The audio gives no LFO line: the log amplitude of harmonics 1 to 10 over 23 to 38 s peaks at 0.33 to 1.7 Hz without a common value (the performance moves the levels), identically in L and R for `k <= 3`.

## 3. Scope reading

The scope shows a **decaying sawtooth of 11.45 ms period** (period of 87.31 Hz) with, before every main edge, a smaller step: the delayed copy. Method (`scope` subcommand, 4 frames per second from 22.5 to 35.5 s, bright-pixel runs):

1. vertical edges = columns with at least 40 bright pixels in a run, merged over adjacent columns;
2. the first edge is followed within 15 to 130 px by the second one (higher on screen); a partner pair one period later (350 to 650 px, same screen row) must exist;
3. `delay_ms = sep_px / period_px * 11.454`; the period of the same trace calibrates the horizontal axis, so camera zoom and tilt cancel to first order.

Results ([data/fx-chorus-hw-scope.csv](data/fx-chorus-hw-scope.csv), 47 rows of the upper trace, 7 of the lower):

* delay series: minima at 22.75, 26.0, 29.0, 32.0, 34.25 s, maxima at 24.5, 27.5, 30.5, 33.0 s: period 2.97 s, **0.336 Hz** (spectral peak), 0.62 to 1.78 ms;
* edge height of the second edge over the first: 0.862 (upper), 0.857 (lower);
* the lower trace follows the upper one within 0.1 ms (7 shared frames: 1.134/1.206, 1.350/1.362, 1.603/1.607, 1.832/1.808, 1.737/1.676, 1.482/1.435, 1.285/1.229 ms).

Limits: a phone filming a screen (perspective, glare, exposure), 4 frames per second against a 3 s cycle (fine), the lower trace overlaps the upper one in some frames (7 rows only). Which trace is the Nord is not certain (the narration says yellow, the frames show white and yellow lines); both agree, so it does not change the numbers. **The sawtooth is not the F2 audio** (the folded F2 waveform of `L.wav` is a smooth low-passed piano shape with harmonic amplitudes `0.149 0.030 0.056 0.013`), so the scope signal is a separate test tone whose frequency and source are not in the clip; the 87.31 Hz calibration is an assumption (ASSUMED: the scope period is the F2 note period; a different frequency scales every delay).

## 4. Emulator on the same sawtooth

`examples/fx_chorus_probe` runs a mono file through one `LayerFxChain` (every other stage off) with `fw4`, `fw5`, `engine`, `engine5` at 48 kHz; `fx_chorus_clip.py grid` feeds it the same sawtooth (87.31 Hz, `exp(-3 p)` decay) and measures the same two-edge separation ([data/fx-chorus-probe-grid.csv](data/fx-chorus-probe-grid.csv), per output channel).

| Setting | Delay min / max / mean (ms) | LFO (Hz) | Second over first edge |
|---|---|---|---|
| `fw4` v 30, amount 8 / 16 / 32 / 48 | 0.87 to 3.28, mean 2.02 | 0.343 | 0.08 / 0.17 / 0.37 / 0.59 |
| `fw4` v 30, amount 64 / 90 / 127 | 0.87 to 3.29 / 0.89 to 3.63 / 1.08 to 5.04 | 0.343 | 0.82 / 0.88 / 0.99 |
| `fw4` v 10 / 60, amount 127 | 1.06 to 4.79 / 0.93 to 5.17 | 0.131 / 0.652 | 0.99 |
| `fw5` v 30, amount 127 | no edge pair | | |
| engine, 0.34 Hz, amount 64 / 127 | 6.7 to 8.8 / 0.7 to 8.4 (the detector window ends at 9 ms; the engine swing is 4.5 to 13.5 ms) | 0.340 | 0.34 / 1.0 |

* **Stereo:** the firmware stage returns the delayed copy on the right output only for a mono input (`ch 1`); the left output is the dry signal plus small spikes (`h` ratio 0.00 to 0.02). The clip is two mono recordings, so it cannot show whether the hardware chorus is stereo.
* **Type 5** (variation) gives a dispersed bump and dip after the edge, not a clean second edge; the clip scope shows a clean edge, so type 4 (non-variation) is the type that agrees, but the variation was not excluded by a second clip (the Nord knob state is unknown).
* The firmware swing is nearly amount independent below amount 64 (`c[-0x1E]` 117 to 156 frames, `c[-0x1D]` 171 to 180 frames), so the delay range cannot be fitted by the amount; only the edge height ratio can (amount near 80 for 0.86).
* Rate: `12800 + 4096 v` for `v = 30` is 135 680, 0.343 Hz; the hardware 0.336 Hz is within 2 %; the clip fixes one rate point, the other ten of the law stay as before (11 captured samples, EMPIRICAL between).

## 5. Implications

* The type 4 rate law and the dry plus delayed copy structure (one clean copy, wet 0.86 of dry) agree with the hardware; the engine chorus (9 ms centre) does not (7.7 times too long if the 87.31 Hz calibration holds).
* The absolute delay of the firmware stage is 1.7 times the clip's. Candidates: the scope frequency differs (a 150 Hz saw would match), the engine-rate rescale of the delay words (`delay_scale = fs / F`, ASSUMED), the hardware layer row (piano words used for every layer, ASSUMED), or the Nord knob state (the swing words come from the amount law between samples).
* D22 asks for band and modulation metrics with no side worse than 3 dB: the delay misses by a factor 1.7 (swing 2.1) and the clip has neither a dry reference nor an audio comb, so no band metric exists to compare; the flip is not justified. `FirmwareFx::mod2` stayed out of `DEFAULT` at that time; **update 2026-11-18 (D163):** the owner's approval, relayed in chat, flipped it on the oracle null (all 12 types) and the ranked distance; the delay miss of 1.7 stays open.

## 6. Update 2026-11-15: channel roles and the UP2 chorus settings

Sources: `clip.srt` (narration subtitles of the clip), `docs/research/nki-lz-stream.md` (UP2 script tables, `tools/ksp_arrays.py`), and KONTRA (github.com/DerpcatMusic/KONTRA, Apache-2.0, `crates/format/kontakt-effects/src/choral.rs`, `phasis.rs`), a third-party reimplementation whose comments cite Kontakt 8.13.1 decompilation. KONTRA values are a third-party claim, not checked by this repo against Kontakt (ASSUMED until a Kontakt readout confirms).

* **Channel roles (from the narration, user-confirmed):** `L.wav` is the Nord Stage 4 chorus recording, `R.wav` is Ultimate Pianos 2 (Kontakt) playing the same notes; on the scope the yellow trace is the Nord and the purple one is UP2. The UP2 library was sampled from the Nord Stage 4. This replaces the section 2 inference that both channels are "wet" without a reference: both are wet, one is hardware, one is the Kontakt chorus.
* **UP2 chorus knob table:** the script sets Choral rate from `%idia3[pos]` (0 to 691137 of 1,000,000, 21 positions) and amount from `%gg3op[pos]`, with delay 30000, width 1,000,000 and mix `level * 46000`. KONTRA's law `rate_Hz = 7.4 r^3 + 0.25` with `r = value / 1e6` gives 0.250 Hz at positions 0 and 1, 0.333 at 2, 0.478 at 3, 0.602 at 4, 1.0 Hz at 7, 2.69 Hz at 20.
* **Check against the clip:** the scope LFO of section 3 is 0.336 Hz; position 2 gives 0.333 Hz (0.9 % apart, nearest of the 21 positions), and Nord type 4 word `v = 30` gives 0.343 Hz. The two rates agree, which supports "UP2 reproduces the Nord rate at the lowest nonzero knob position". It is one data point, with the scope calibration still ASSUMED (87.31 Hz), so it is EMPIRICAL support, not VERIFIED.
* **L (Nord) against R (UP2), 23 to 37 s, 8192-point windows, mean over the note** (scratch script, not committed; run again with the two WAVs):

  | Band (Hz) | L - R level (dB) | Mean coherence |
  |---|---|---|
  | 60 to 200 | 0.00 | 0.997 |
  | 200 to 500 | -0.15 | 0.885 |
  | 500 to 1000 | -0.21 | 0.880 |
  | 1000 to 2000 | -1.01 | 0.695 |
  | 2000 to 4000 | +0.64 | 0.721 |
  | 4000 to 8000 | -0.11 | 0.525 |
  | 8000 to 16000 | +0.03 | 0.533 |

  The long-term spectrum of the two chorus recordings agrees within 1 dB in every band, so the UP2 sample set plus its Kontakt chorus has the Nord level balance (consistent with UP2 being sampled from the Nord). The coherence drop with frequency is what two choruses with independent LFO phase produce; it does not measure the delay depth. The harmonic amplitude tracks (harmonics 1 to 18 over 23 to 37 s) are within 1 dB of each other too and show no LFO line in either channel (the performance dominates), so the LFO rate still rests on the scope reading.
* The Phasis rate law of KONTRA (`440 * 2^((lerp(-120, 20, r) - 69) / 12)` Hz) gives 0.051 Hz to 12.2 Hz over the `%pwbo1` table; Flair's rate law is unread in KONTRA.

## 7. UP2 table positions against the emulator FX (2026-11-15, no Kontakt run)

Source of the UP2 side: `tools/ksp_arrays.py` on the UP2 script (`arrays.csv`, local). Source of the Hz and ms laws for the UP2 chorus: KONTRA `choral.rs` (third party, not run, not downloaded; **ASSUMED**). Emulator side: `fw_mod2.rs` (rate `12800 + 4096 v`, `F = 43,402.34375 Hz`, `Hz = word * F / (4 * 2^32)`, VERIFIED words, no hardware recording of the Hz, see [fx-mod2-types](fx-mod2-types.md)). Scratch script, not committed; all numbers below recompute from the two sources.

**Chorus rate.** The type 4 range is 0.0323 to 1.3465 Hz (`v = 0..127`). Table `%idia3` through the ASSUMED law `7.4 r^3 + 0.25` gives 0.250 Hz (positions 0, 1) to 2.693 Hz (position 20):

| UP2 position | 0, 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 to 20 |
|---|---|---|---|---|---|---|---|---|---|---|
| UP2 rate (Hz) | 0.250 | 0.333 | 0.478 | 0.602 | 0.727 | 0.870 | 0.996 | 1.118 | 1.265 | 1.388 to 2.693 |
| nearest type 4 `v` | 21 | 29 | 43 | 55 | 67 | 81 | 93 | 105 | 119 | above range (`v = 127` gives 1.347) |

Positions 0 to 9 fall inside type 4 and step by 12 to 14 `v` each (the 128 `v` steps are about 1.5 % apart, so the 0.2 % residuals say nothing by themselves; the near-uniform spacing is the only signal). Positions 10 to 20 (1.39 to 2.69 Hz) exceed type 4; type 5 spans 0.25 to 4.0 Hz, so either the UP2 upper positions imitate the variation type or the ASSUMED law is wrong above 1.3 Hz. Not decidable without a Kontakt readout.

**Chorus depth and delay.** The UP2 script fixes delay 30000 and width 1,000,000; the amount comes from `%gg3op` (0.263 to 0.929). KONTRA's reading (ASSUMED: mode 0, voice 0 only, `span = 5 * Delay + 0.1`, `R = 2^(0.025 (0.99 a^2 + 0.01))`, delay `G0 * span`, swing `(R - 1) / (2 f)`) gives base 0.75 ms and a swing of 2.75 to 2.80 ms at positions 2, 7, 12, 20, so 0.75 to about 3.5 ms nearly independent of position. The firmware stage at `v = 30` gives 0.87 to 3.28 ms (section 4), the clip scope 0.62 to 1.78 ms. The UP2 reading and the firmware agree within about 15 % at the low end and 8 % at the high end; the scope is 1.7 times shorter on the swing (section 5), so the scope calibration (ASSUMED 87.31 Hz) stays the weak point, not the firmware.

**Phaser and flanger.** The UP2 phaser rate table `%pwbo1` through KONTRA's law (ASSUMED) gives 0.051 to 12.2 Hz. The firmware PHAS 0 rate word is a phase increment `0x27CF` (`v = 0`) to `0x256145` (`v = 127`) per DSP call (fx-mod2-types s.8); read as a 32-bit phase accumulator at `F / 4` that is 0.026 to 6.2 Hz (ASSUMED reading, not checked against a recording). The two ranges overlap from 0.05 to 6 Hz; the UP2 positions above 6 Hz have no firmware counterpart. No firmware type was mapped to the UP2 flanger (rate law unread in KONTRA); amounts and feedback tables (`%us0tu`, `%hdvpr`, `%zopuj`, `%01udi`) were not compared.

**EQ `FREQ2`.** The 33 labels of `!ekn0u` (200 Hz to 8 kHz) pair with `%liluo` (333,333 to 867,353). `20 * 1000^(v / 1e6)` Hz fits all 33 points within 6.0 % (exact at 333,333 and 767,010: 200 and 4000 Hz); the 6 % is label rounding of the display steps, so the law is EMPIRICAL. The emulator mid sweep spans about 0.2 to 6.8 kHz (`fx-eq-words.md`, VERIFIED words, ASSUMED joint law): the same decade as the UP2 labels, but the endpoints differ (6.8 against 8 kHz); the 200 Hz bottom matches.

**What this does not give.** No emulator output was compared with a UP2 render: that needs the official Kontakt Player or a Kontakt readout of `CHORAL_RATE`, `CHORAL_DELAY` and `get_engine_par_disp`. Reverb has no IR here (the `.nkr` `ir_samples` folder is empty).

## Follow-ups

1. From the user: the Nord Mod 2 settings of the recording (Rate knob position and the Amount knob, VARIATION on or off, program or sound, Mod 2 B and which layer), the plugin knob values, and what the scope shows (the signal source, its frequency, the time axis of the scope window).
2. One more clip with chorus off then on (or a dry reference recorded in the same take) so the audio comb can be fitted, not only the scope.
3. A stereo recording (left and right outputs of the Nord) to settle the right-only wet of the port.
4. Re-run `uv run python tools/fx_chorus_clip.py grid` with the corrected frequency when the scope frequency is known.

## Reproduce

```bash
cargo build --release -p nord-core --example fx_chorus_probe
uv run python tools/fx_chorus_clip.py scope .reference/chorus-nord-vs-ultimate-pianos2/clip.mp4 --out docs/research/data/fx-chorus-hw-scope.csv
uv run python tools/fx_chorus_clip.py stats docs/research/data/fx-chorus-hw-scope.csv
uv run python tools/fx_chorus_clip.py grid --workdir <tmp> --out docs/research/data/fx-chorus-probe-grid.csv
uv run pytest tools/tests/test_fx_chorus_clip.py
```
