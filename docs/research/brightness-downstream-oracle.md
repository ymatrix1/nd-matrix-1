# Downstream stages against the oracle: where the 3 to 5 dB treble excess does not come from, and the one stage error that adds to it

- **Checked:** 2026-11-17
- **Sources (local, never committed):** the FX Core 1 flat SRAM image (`tools/ns4_dsp_ldr.py --sram` of the v1.66 FX image), `sharc-fxlive` and `tools/fx_oracle_live.py` ([fx-oracle-live](fx-oracle-live.md), D125, D133), the new `fx_stage_response` (`crates/nord-tools/src/bin/fx_stage_response.rs`, `band_response.rs`), the oracle-verified ports under `crates/nord-core/src/fx/` (`tests_amp_golden.rs`, `tests_cabinet_golden.rs`, `tests_resonant_golden.rs`, `tests_compressor_golden.rs`, `tests_master_golden.rs`), [dsp-frame-rate](dsp-frame-rate.md), the Part A demo clips through `tools/emu_vs_hw.py` ([emulator-vs-hardware-audio](emulator-vs-hardware-audio.md), [hardware-audio-reference-fx](hardware-audio-reference-fx.md)), the factory programs of the clips (`ns4 backup --extract`, `ns4 convert --to-json`; only parameter numbers are quoted here). No firmware byte, sample or audio is printed or committed.
- **Question:** the emulator reads 3 to 5.5 dB brighter than the hardware recordings above 1.5 to 3 kHz on clips of different engines. Is the cause a stage downstream of the engines (amp models, Amp EQ, compressor, master or output stage), and where do the oracle and the native stage differ by 3 dB or more?

Tags follow `.agents/rules/provenance-in-code.md`.

## 1. Short answer

| Item | Result | Tag |
|---|---|---|
| Master / output stage in the oracle FX chain | White noise through the live FX chain with a pass-through stage 0 (`fx_oracle_live.py identity --signal noise`) comes out as `0.1866 x` the input, lag 0, **null depth -156.3 dB** after the gain fit, spectral centroid 11,085.2 Hz on both sides. The chain (input stage, 35 stage calls, master fade-in and level) is **flat to the numerical floor**: no low-pass, no shelf, no compressor action at that level. The 4-section master IIR of `master_iir.rs` (PM `0x945F2`, -3.9 dB at 8..16 kHz) is therefore **not in this path**; the native default also has it off. Scope: this is the pass-through FX chain at about -30 dBFS rms noise (rms 0.032). It says nothing about the amp, EQ or compressor (not engaged in that run; judged from the ports), nothing about level-dependent behaviour (compressor knee, soft clip) at hotter levels, and nothing about other places a master filter could live (ARM mixing, the PNO / ORG / SY output stages, the DAC and analog path, the video recording chain) | VERIFIED (run) for the pass-through chain at -30 dBFS only |
| Amp drive and cabinet | The firmware port is the default (`FirmwareFx::DEFAULT.amp`) and bit exact per frame against the oracle. It runs once per host frame at 48 kHz while the firmware runs once per frame at 43,402.34 Hz, so its response in Hz is stretched by 1.106. Against the response at the frame rate (the oracle's, in Hz): **+0.5 to +2.2 dB in 3..8 kHz, +0.9 to +3.5 dB in 8..16 kHz, 0.3 to 2.0 dB less at 1.5..3 kHz; 60 of 234 third-octave cells (18 amp setups x 13 bands) differ by 3 dB or more, worst +10.8 dB at 16 kHz** | VERIFIED by inference: the port at the frame rate stands in for the oracle (per-frame equality by the golden tests; frame rate VERIFIED in [dsp-frame-rate](dsp-frame-rate.md)); the oracle itself was not driven with noise through the amp (follow-up 4) |
| Engine amp path (`--amp-path engine`) against the firmware stage | 7 to 10 dB less presence and brilliance for Twin, 3 to 10 dB for Small II, large level differences with drive; the engine path is not the default, so it is not the excess | VERIFIED (run) |
| Amp EQ | Engine biquads against the firmware tone stack `0x1243F4` at the frame rate: at most **1.7 dB** (treble +6 dB, 1.5..3 kHz), 0.9 dB for the Tiny Transistor setting; running the tone stack at 48 kHz instead of 43,402 Hz changes it by at most 0.6 dB | VERIFIED (run) |
| Layer compressor | The firmware gain computer is a wideband gain (no frequency dependence: identical gain in 1.5..3, 3..8, 8..16 kHz at amounts 0.25, 0.5, 1); the engine's peak-envelope compressor differs in level (1.6 to 10.8 dB), not in tone | VERIFIED (run) |
| Master bus (native) | Default reconstruction low-pass plus level law: +0.2 to +0.3 dB flat above 1.5 kHz; the firmware IIR would be -3.9 dB at 8..16 kHz at 48 kHz (-5.7 dB at the frame rate) | VERIFIED (run) |
| `LP24` / `HP24` / To Rotary | **Cannot be run in the oracle.** The type switch sends no words into the drive stage context for types 7 to 9 ([fx-type-map](fx-type-map.md) s8), and the organ unit's tone stack instance and the table `0x2f730c` consumer are unknown ([fx-eq-words](fx-eq-words.md) s5). The engine LP24 / HP24 (two Butterworth biquads) is `ASSUMED` | evidence: [fx-type-map](fx-type-map.md) s8, [fx-eq-words](fx-eq-words.md) |
| Opt-in fix | `--fx-rate-match` / `LayerFxChain::set_fx_rate_match(true)`: the firmware amp stage runs behind a sinc converter at its own frame rate. Response in Hz within **0.13 dB** (aggregate bands) and **1.09 dB** (worst of 234 third-octave cells; 0 cells at 3 dB) of the frame-rate response. Default off | VERIFIED (run, tests) |

**Verdict.** No downstream stage that the oracle can run produces a 3 to 5 dB excess by itself. The piano clip, which uses no downstream stage, carries +3.2 dB presence and +6.3 dB brilliance on current `main`; on the Farfisa clip +10.3 of the +13.1 dB brilliance remain with every program stage switched off (section 4.2). Current `main` also reads larger excesses on the two amp clips (+13.1 and +12.7 dB) than the 3 to 5.5 dB of the 2026-11-11 and 2026-11-13 tables, which pre-date the firmware organ level and routing work. The single measurable stage error, the frame-rate mismatch of the firmware amp, adds 0.7 to 2.2 dB of brilliance on amp programs and is removed by the opt-in. The common excess is not an amp, EQ, compressor or master error. Section 5 gives what remains (source side, `LP24`).

## 2. Method

1. **Oracle chain.** `tools/fx_oracle_live.py identity --signal noise --seconds 1.0` (3000 silent lead passes so the master fade-in has settled): stage 0 is the pass-through routine `0x1231CB`, master level 1.0. The output is compared with the input scaled by the best-fit gain.
2. **Stage responses.** The routines of the stages are ported and bit exact per frame against the oracle (golden vectors, tolerances in the tests named above), so the oracle's response of a stage in Hz is the port run one call per frame at **43,402.34 Hz**. `fx_stage_response` drives each stage with the same deterministic white noise (`band_response::noise`, stereo decorrelated, -30 dBFS rms for amps and EQ, -18 dBFS for the compressor, 131,072 frames), takes the Hann FFT power of input and output and reports the gain in dB per band:
   - `fwF`: port at 43,402.34 Hz (the oracle's response in Hz);
   - `fw48`: the same port at 48 kHz (what a 48 kHz render does);
   - `rm48`: `fw48` behind `set_fx_rate_match(true)`;
   - `eng48`: the engine path at 48 kHz.
   `d48 = fw48 - fwF`, `dRM = rm48 - fwF`, `dEng = eng48 - fwF`. Aggregate bands are the scoring bands of `video_ref.py` (1.5..3, 3..8) plus 1..1.5 and 8..16 kHz; `--detail` prints 13 third octaves from 1 to 16 kHz. Cases: Twin, JC, Small, each as type I and II, at drive 0, 0.11 (Vox J3) and 0.37 (Tiny Transistor); five EQ settings; three compressor amounts; four master configurations.
3. **Clips.** The factory program of each clip is converted to JSON and its amp, EQ, compressor, reverb and master compressor fields are read (section 4). `tools/emu_vs_hw.py --confirmed-only` renders the same notes with the default engine and with the opt-in (`docs/research/data/emu_vs_hw_rate_match.json`), and ablates one stage at a time with program edits (`emu_vs_hw_downstream_ablation.json`). Results: `emu_vs_hw_rate_match_results.json`, `emu_vs_hw_downstream_ablation_results.json`.

Reproduce:

```bash
cargo build --release -p nord-core -p nord-sharc -p nord-tools
target/release/fx_stage_response            # aggregate bands; add --detail for 13 third octaves
python tools/ns4_dsp_ldr.py <FX image> --sram <dir>
python tools/fx_oracle_live.py identity --image <dir>/core1_sram.bin --signal noise --seconds 1.0
uv run python tools/emu_vs_hw.py --spec docs/research/data/emu_vs_hw_rate_match.json --video <Part A .webm> --backup <factory .ns4b> --workdir <scratch> --confirmed-only
```

## 3. Per stage, above 1 kHz

### 3.1 Master / output (oracle)

| Quantity | Value |
|---|---|
| Input L rms / output L rms (noise burst, 0.1 s) | 0.032202 / 0.006009 = 0.1866 x |
| Best lag | 0 frames |
| Null depth after the gain fit | -156.3 dB |
| Centroid oracle / native (pass-through) | 11,085.23 / 11,085.23 Hz |

A white-noise burst covers 0 to 21.7 kHz, so a shelf, a low-pass or a compressor knee anywhere above 1 kHz would show as a null depth of -40 dB or worse. There is none. Native: `MasterBus::default` is `+0.31 / +0.28 / +0.19 dB` at 1.5..3 / 3..8 / 8..16 kHz (the level law `1 + 0.15 x amount` below the knee, amount 0.25); with `master_iir_enabled` it is `+0.31 / +0.28 / -3.90 dB` at 48 kHz. The IIR is opt-in and **not evidenced as part of the hardware chain**: its position was ASSUMED (`master_iir.rs`) and the oracle chain does not contain it. No change.

### 3.2 Amp drive and cabinet (Twin, JC, Small, drive 0.11 and 0.37)

Gain in dB, noise -30 dBFS rms. `d48` is native default (firmware port at 48 kHz) minus oracle in Hz; `dRM` the same with the opt-in.

| Setup (clip) | Band | fwF (oracle) | fw48 | rm48 | d48 | dRM |
|---|---|---|---|---|---|---|
| Twin I drive 0.11 (Vox J3) | 1.5..3 k | 12.21 | 10.88 | 12.30 | -1.33 | +0.09 |
| | 3..8 k | 12.81 | 13.53 | 12.88 | +0.72 | +0.07 |
| | 8..16 k | -3.96 | -1.22 | -3.99 | +2.73 | -0.03 |
| Small II drive 0.37 (Tiny Transistor) | 1.5..3 k | 11.80 | 11.47 | 11.82 | -0.33 | +0.02 |
| | 3..8 k | 3.23 | 5.34 | 3.35 | +2.12 | +0.13 |
| | 8..16 k | -18.87 | -15.42 | -18.94 | +3.45 | -0.06 |
| Small II drive 0 | 3..8 k | -0.71 | 1.45 | -0.62 | +2.16 | +0.10 |
| Twin II drive 0.37 | 8..16 k | -9.76 | -6.68 | -9.83 | +3.09 | -0.07 |

Pure tones through the Small II amp (`tests_fw_rate_match_chain.rs`, amplitude 0.03): the 48 kHz chain differs from the frame-rate chain by +4.1 dB at 3 kHz, -4.5 dB at 5 kHz, +4.5 dB at 6 kHz, +8.4 dB at 7 kHz and +7.5 dB at 9 kHz (cabinet slope features moved by 10.6 %); the rate-matched chain is within 0.01 dB of the frame-rate chain at every tone of the scan (2 to 15 kHz). 60 of 234 third-octave cells exceed 3 dB at 48 kHz, none with the rate match.

The engine path is not the default; against the oracle it is `dEng = -10.2 / -7.0 / +8.9 dB` (Twin I drive 0) at 1.5..3 / 3..8 / 8..16 kHz, a different curve altogether.

### 3.3 Amp EQ

| Setting | Band | fwF | fw48 | eng48 | d48 | dEng |
|---|---|---|---|---|---|---|
| Tiny Transistor (bass +8.75, mid -1.5 at 1350 Hz, treble -5) | 1.5..3 k | -2.15 | -2.11 | -1.27 | 0.04 | 0.87 |
| | 3..8 k | -3.54 | -3.36 | -3.59 | 0.18 | -0.06 |
| Treble +6 dB | 1.5..3 k | 2.36 | 2.10 | 0.66 | -0.26 | -1.70 |
| | 3..8 k | 4.72 | 4.49 | 4.48 | -0.23 | -0.24 |
| A:25 (mid -6 dB at 1500 Hz) | 1.5..3 k | -2.98 | -3.58 | -3.43 | -0.60 | -0.45 |

The default engine EQ (three RBJ biquads) is within 1.7 dB of the oracle tone stack in every aggregate band (engine biquads against the firmware recurrence at the frame rate); the firmware recurrence itself run at 48 kHz instead of the frame rate is within 0.6 to 0.7 dB of the same reference (the two figures compare different things): no 3 dB difference. `FirmwareFx::eq` stays opt-in; the 1.5 to 3 kHz shelf difference of 1.7 dB (engine under the firmware for a treble boost) is in the direction that makes the emulator *darker*, not brighter.

### 3.4 Layer compressor

Firmware gain computer against the engine's: identical gain in all three aggregate bands (a wideband gain), `dEng = -1.63 / -5.30 / -10.80 dB` at amounts 0.25 / 0.5 / 1 (level only). A compressor cannot tilt the spectrum of a steady noise; it only matters through level, which the clip comparison already normalises (bands are re RMS). Only A:25 enables it (amount 0.5).

## 4. What the six clips exercise

Program fields (factory v1.66; `master_compressor` is 0 on all six):

| Clip | Program | Amp mode (type) | Amp drive | EQ bass / mid / treble dB, mid Hz | Layer compressor | Other FX |
|---|---|---|---|---|---|---|
| piano_dry | A:61 Velvet Grand | none | 0 | 0 / 0 / 0 | off | none (reverb, mod, delay off) |
| A:25 | Sidechain Whl II | **LP24** piano (424 & 15 = 8), HP24 on synth B (457 & 15 = 9) | 0 (piano), 1.57 (synth B) | piano: 0 / -6 / 0 at 2000 Hz (LP24: cutoff 2000 Hz, resonance 3); synth B: mid -11.5 | piano **on, 0.5** | reverb 0.5, delay on |
| Tiny Transistor | A:64 | Small II (118 & 15 = 6) | 3.7 | +8.75 / -1.5 / -5.0 at 1500 Hz | off | mod 2 type 3, reverb 0.2 |
| Vox J3 | A:84 | Twin I (289 & 15 = 1) | 1.1 | -3.0 / 0 / 0 | off | mod 2, delay, reverb 0.24 |
| Smooth Lead | A:87 | none | 0 | 0 | off | reverb 0.3 |
| B3 Chords n Solo | A:14 | none | 0 | 0 | off | reverb 0.22 |

So: **piano_dry, Smooth Lead and B3 use no amp, EQ or compressor, yet two of them show the excess** (piano: presence +3.2, brilliance +6.3 dB; Smooth Lead is inside 2 dB). Their only downstream stage is the master bus, which the oracle chain shows to be flat. The Farfisa clip and the Vox clip use the firmware amp (the d48 stage error applies); A:25 uses `LP24` / `HP24`, which the oracle cannot run.

### 4.1 Before and after (`--fx-rate-match`), confirmed notes, emulator minus hardware, dB re RMS

Current `main`, `tools/emu_vs_hw.py --confirmed-only` (`emu_vs_hw_rate_match_results.json`; the older tables of [emulator-vs-hardware-audio](emulator-vs-hardware-audio.md) pre-date the firmware organ level and routing work, so absolute numbers moved).

| Clip | Variant | mid 0.4..1.5k | pres 1.5..3k | brill 3..8k | air 8..20k (not scored) | centroid Hz | score |
|---|---|---|---|---|---|---|---|
| fx_amp (Tiny Transistor, Small II) | default | -2.7 | +4.8 | **+13.1** | +13.7 | +766 | 5.80 |
| | `--fx-rate-match` | -2.4 | +5.2 | **+8.7** | +10.1 | +631 | 4.80 |
| organ_vox (Vox J3, Twin) | default | +8.1 | -1.3 | +2.3 | +6.5 | +292 | 3.03 |
| | `--fx-rate-match` | +6.8 | +1.2 | +1.9 | +3.9 | +491 | 2.95 |
| | `--tone-source vox` | +7.7 | -7.1 | -9.4 | -25.0 | -1082 | 5.86 |
| | `vox` + `--fx-rate-match` | +7.3 | -5.3 | -9.3 | -27.9 | -1043 | 5.46 |
| piano_dry (no FX) | default = `--fx-rate-match` | -1.9 | +3.2 | +6.3 | +4.4 | -31 | 2.85 |
| A:25 (LP24 + HP24) | default = `--fx-rate-match` | +0.1 | +4.9 | **+12.7** | +12.0 | -2 | 4.49 |
| B3, Smooth Lead | default = `--fx-rate-match` | unchanged | | | | | 5.42, 0.69 |

The rate match changes only the clips with a firmware amp (bit-identical elsewhere, as designed). On the Farfisa clip it takes 4.4 dB off brilliance, more than the 2.1 dB of the noise measurement because the organ spectrum falls steeply across the cabinet's edge; the score improves 5.80 to 4.80. On the Vox clip the effect is small (presence moves from -1.3 to +1.2 dB, the score 3.03 to 2.95), and the Vox bank variants stay far darker than the recording. Neither clip reaches 0 dB, and the rate match does nothing for the piano and A:25 excess.

### 4.2 Stage ablation (`emu_vs_hw_downstream_ablation_results.json`)

Bands are dB re RMS, emulator minus hardware, confirmed notes; each variant switches one program stage off (edits) or one opt-in firmware port on (flags). Because bands are relative to RMS, removing a stage that adds a lot of low-frequency level (the amp) moves every band; read the spectrum shape, not single cells.

| Clip | Variant | sub | low | mid | pres 1.5..3k | brill 3..8k | air (not scored) | RMS | score |
|---|---|---|---|---|---|---|---|---|---|
| Tiny Transistor | default | +1.2 | -7.3 | -2.7 | +4.8 | +13.1 | +13.7 | -0.6 | 5.80 |
| | `--fx-rate-match` | +1.8 | -6.0 | -2.4 | +5.2 | +8.7 | +10.1 | -0.6 | 4.80 |
| | amp off | +11.0 | +2.7 | -2.0 | +2.0 | +10.3 | +23.1 | +1.6 | 5.60 |
| | EQ flat | -4.6 | -8.6 | -3.2 | +5.0 | +13.9 | +17.4 | +0.7 | 7.05 |
| | amp, EQ, reverb, mod 2 all off | +11.3 | +2.7 | -2.0 | +2.0 | +10.3 | +23.4 | +2.5 | 5.64 |
| | reverb off / mod 2 off | +1.3 / +1.2 | -7.1 / -7.5 | -2.6 / -2.7 | +4.8 / +4.8 | +13.2 / +13.1 | | -0.2 / -0.5 | 5.80 / 5.86 |
| | firmware EQ (`--firmware-fx-ports eq`) | +3.4 | -4.9 | -2.1 | +4.1 | +12.7 | +14.6 | -0.7 | 5.45 |
| Vox J3 | default | -3.2 | -0.2 | +8.1 | -1.3 | +2.3 | +6.5 | +8.9 | 3.03 |
| | `--fx-rate-match` | -3.6 | -1.3 | +6.8 | +1.2 | +1.9 | +3.9 | +9.7 | 2.95 |
| | amp off | -2.0 | -0.1 | +13.8 | -6.7 | -11.6 | +7.2 | +9.6 | 6.86 |
| | amp, reverb, mod 2, delay off | -2.5 | -0.1 | +14.3 | -6.6 | -11.4 | +6.9 | +11.5 | 6.97 |
| A:25 | default | +3.4 | +1.3 | +0.1 | +4.9 | +12.7 | +12.0 | +4.0 | 4.49 |
| | LP24 and HP24 off | +3.5 | +1.4 | -0.1 | +3.8 | +24.8 | +22.0 | +3.9 | 6.74 |
| | layer compressor off | +3.4 | +1.3 | +0.1 | +4.9 | +13.0 | +4.8 | +3.6 | 4.55 |
| | firmware compressor (`--firmware-fx-ports compressor`) | +3.4 | +1.3 | -0.1 | +5.0 | +9.4 | +5.3 | +8.8 | 3.85 |
| | firmware EQ / reverb off | unchanged / +3.5 | | | +4.9 / +5.0 | +12.7 / +12.9 | | | 4.49 / 4.55 |

Reading:

- **Tiny Transistor.** With every downstream stage off the clip is still +10.3 dB in brilliance and +23 dB in air: three quarters of the excess is the Farfisa source (not a stage). The amp adds the remainder (rate-matched: brilliance 13.1 to 8.7 dB, more than the 2.8 dB that the amp-off row suggests, because that row also moves the RMS reference by +2.2 dB and the low bands by +10 dB). The EQ is in the right direction (flat is worse, 7.05) and the firmware EQ is slightly better (5.45), as the 1.7 dB oracle difference predicts; reverb and Mod 2 are inert (under 0.4 dB).
- **Vox J3.** The amp is what makes this clip bright enough (brilliance -11.6 dB without it, +2.3 with); the clip is not over-bright through the amp, so the rate match moves presence from -1.3 to +1.2 dB and changes the score by 0.08.
- **A:25.** `LP24` / `HP24` take 12 dB off brilliance (24.8 to 12.7) and still leave +12.7 dB: the engine filters do their job in direction but the clip is far from the recording, and the oracle cannot say whether the topology (`ASSUMED`) is the reason. The layer compressor is a level effect (air changes 7 dB because the peak-envelope law rides the loud top end). The **firmware compressor** (opt-in, `kCmpAmt` words VERIFIED, slope and log gain `ASSUMED`) lowers brilliance by 3.3 dB and the score from 4.49 to 3.85 while raising the RMS gap by 4.8 dB: a lead, not a result (the RMS gap shows the makeup law is not calibrated against this clip).


## 5. Implications

- **There is no shared downstream stage behind the 3 to 5 dB.** The oracle FX chain is flat (-156.3 dB null on noise), the layer compressor has no tone, the Amp EQ is within 1.7 dB of its oracle, and the clip that is closest to a bare engine (piano_dry: no amp, EQ, compressor, reverb, mod or delay) carries the same excess (+6.3 dB brilliance). The excess in that clip is the piano source level and brightness (velocity: [piano-downstream-level](piano-downstream-level.md) s3 measures brilliance moving by 20 dB between velocity 100 and 64), not a stage.
- **The frame-rate mismatch of the firmware amp is real and cheap to remove.** A firmware-ported FX stage that carries per-frame coefficients or taps has the same defect when it runs at 48 kHz, and this audit did not cover every such stage: the firmware reverb (`FwReverb`, `FwReverbMulti`: delay lengths in frames are about 10% short in time at 48 kHz and its damping filters have the same frequency stretch) and the compressor time constants do not use `FIRMWARE_FRAME_RATE_HZ` and are not measured here (follow-up 5); the amp drive and cabinet are the default path and the audible one (+0.7 to +2.2 dB brilliance by noise, +4.4 dB on the Tiny Transistor clip). The Mod 1 and Mod 2 stages already rescale their rates (`FIRMWARE_FRAME_RATE_HZ / fs`); the Amp EQ tone stack is within 0.6 dB; the delay uses the frame rate law (`fw_delay_time`). The rate-matched amp is **opt-in**: it adds about 0.8 ms of latency, 2 sinc kernels of cost per host frame on amp layers, and the owner decides whether the default flips (D22; the numbers are in section 4.1).
- **A:25 is unexplained and its main unverified downstream stage is `LP24` / `HP24`.** The clip's presence +4.9 and brilliance +12.7 dB pass through an engine `LP24` (cutoff 2000 Hz, resonance 3, two Butterworth biquads, `ASSUMED`) that the oracle cannot run; removing it makes the brilliance 24.8 dB, so the topology is at least not far in the wrong direction, but 12.7 dB remain: the Super Saw / piano sources, the `HP24` on the synth layer or the real LP24 slope. The firmware compressor opt-in improves this clip (4.49 to 3.85) and is the one downstream lead with the right sign and size (3.3 dB of brilliance), not yet understood (RMS +4.8 dB).
- The master IIR (`master_iir_enabled`) has no support from the oracle chain and stays off.

## Follow-ups

1. Find the `LP24` / `HP24` words: the consumer of the 18 x 128 table `0x2f730c` and the organ unit's tone stack instance ([fx-eq-words](fx-eq-words.md) s5), then run them in the oracle and compare with the engine two-biquad model; A:25 is the clip.
2. Decide the default of `--fx-rate-match` (and its latency) with the owner; if it flips, apply the same converter to the Amp EQ tone stack (error under 0.6 dB, not worth it alone) and re-run `render_hash`.
3. Piano: a velocity-calibrated replay of piano_dry (the stage-free clip) before attributing any brilliance to the source model.
4. A direct oracle run of the amp stage on noise (not only the per-frame golden equality) once `tools/fx_amp_cabinet.py chain` accepts a noise input; it would remove the one inference of section 2 (port equals oracle per frame, hence equal response in Hz at the frame rate).
5. Check the firmware reverb (delay lengths, damping filters) and the compressor time constants for the same per-frame versus host-rate mismatch; they are not in the stage table of this note.
6. Premise: the 3 to 5 dB of the task brief predates the organ level work; on current main the clips read +13.1 (Tiny Transistor), +12.7 (A:25) and +6.3 dB (piano_dry) in brilliance. `--fx-rate-match` removes 4.4 dB on one clip and about 0.1 dB on Vox (Vox presence moves from -1.3 to +1.2 dB, brighter); three clips still read +6 to +13 dB, so the excess is not largely explained by this change.
