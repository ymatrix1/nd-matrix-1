# Research: factory program coverage and render health

- **Checked:** 2026-11-17
- **Sources (local, never committed):** the factory restore backup `Nord Stage 4 Factory Restore RevK v1.66.ns4b` (384 `.ns4p` programs, its `.npno` and `.nsmp4` entries), `tools/program_sweep.py` driving `crates/nord-tools/src/bin/program_sweep.rs` (release build), the classification of paths from `docs/specs/*.md` and `docs/research/*.md` cited per row.
- **Question:** how much of the factory program space does the emulator cover, which engine paths do the 384 programs rely on, and which of them are firmware-verified, assumed or not modelled; does any program panic, go non-finite, fall silent or clip.

This is a coverage and risk map, not an audio-quality claim. All numbers are derived from the decoded programs and from output statistics; no audio, firmware or sample content is stored.

## 1. Method

- Layer used = bit set in the scene I or scene II mask the program stores (section enable AND layer enable, `ProgramRouting::exact_masks`).
- Render: `96000` frames at 48 kHz, notes 48, 55, 64, 72 (C3, G3, E4, C5) velocity 100, released at frame 72000, one engine per render, release build. The same note set drives every program, so a layer whose key zone excludes all four notes counts as silent here.
- `default` = engine defaults. `firmware` = `--firmware-all --firmware-routing` (firmware tone source, filter, strip chain, FX ports, piano level, program routing).
- Output health: NaN/Inf counted, silent = peak < 1e-4, clipped = |x| >= 1.0; clipping is reported at >= 0.1% of samples, level jump at >= 6 dB RMS difference.
- Status per path: `verified` = a firmware-verified algorithm runs; `assumed` = engine approximation or legacy model; `unmodelled` = pass-through, placeholder or decoded and ignored. The classification is a reading of the cited specs, not a measurement.

## 2. Engine usage

| Engine mix | Programs |
|---|---|
| piano+synth | 173 (45%) |
| synth | 90 (23%) |
| piano | 77 (20%) |
| organ+piano+synth | 16 (4%) |
| organ | 12 (3%) |
| organ+synth | 10 (3%) |
| organ+piano | 6 (2%) |

| Layers sounding | Programs |
|---|---|
| 1 | 99 |
| 2 | 111 |
| 3 | 90 |
| 4 | 63 |
| 5 | 18 |
| 6 | 2 |
| 7 | 1 |

Scene II used by 213 programs, split enabled in 48, non-default key zone code on a sounding layer in 172.

| Key zone code (sounding layers) | Programs |
|---|---|
| 0 | 40 |
| 1 | 4 |
| 10 | 3 |
| 2 | 2 |
| 3 | 1 |
| 4 | 9 |
| 5 | 256 |
| 6 | 37 |
| 9 | 119 |

## 3. Organ

| Path | Programs | Occurrences |
|---|---|---|
| `organ.model=B3` | 27 | 36 |
| `organ.model=B3 Bass` | 4 | 4 |
| `organ.model=Farfisa` | 2 | 3 |
| `organ.model=Pipe1` | 3 | 4 |
| `organ.model=Pipe2` | 4 | 5 |
| `organ.model=Vox` | 7 | 8 |
| `organ.rotary_speed=Fast` | 1 | 2 |
| `organ.rotary_speed=Slow` | 18 | 27 |
| `organ.rotary_speed=Stop` | 5 | 6 |
| `organ.vibrato_type=0` | 1 | 1 |
| `organ.vibrato_type=1` | 3 | 4 |
| `organ.vibrato_type=2` | 2 | 2 |
| `organ.vibrato_type=4` | 3 | 3 |
| `organ.vibrato_type=5` | 12 | 15 |
| `organ.percussion` (enabled) | 6 | 8 |
| `organ.rotary` (enabled) | 24 | 35 |
| `organ.vibrato` (enabled) | 21 | 25 |

## 4. Piano

| Path | Programs | Occurrences |
|---|---|---|
| `piano.category=Clav` | 12 | 12 |
| `piano.category=Digital` | 14 | 14 |
| `piano.category=Electric` | 59 | 63 |
| `piano.category=Grand` | 153 | 156 |
| `piano.category=Misc` | 8 | 9 |
| `piano.category=Upright` | 50 | 57 |
| `piano.sample=loaded` | 272 | 311 |
| `piano.class=1` | 5 | 5 |
| `piano.class=14` | 10 | 10 |
| `piano.class=15` | 4 | 4 |
| `piano.class=16` | 8 | 9 |
| `piano.class=2` | 52 | 55 |
| `piano.class=3` | 8 | 8 |
| `piano.class=4` | 9 | 9 |
| `piano.class=5` | 150 | 151 |
| `piano.class=6` | 50 | 57 |
| `piano.class=7` | 3 | 3 |
| `piano.string_res` (enabled) | 223 | 244 |

## 5. Synth

| Path | Programs | Occurrences |
|---|---|---|
| `synth.osc=Analog` | 193 | 314 |
| `synth.osc=Fm` | 25 | 38 |
| `synth.osc=Sample` | 119 | 215 |
| `synth.osc=Wavetable` | 12 | 14 |
| `synth.analog=MISC` | 8 | 8 |
| `synth.analog=MULTI` | 43 | 44 |
| `synth.analog=PURE` | 61 | 90 |
| `synth.analog=SHAPE` | 50 | 64 |
| `synth.analog=SHAPE SINE` | 30 | 32 |
| `synth.analog=SUB OSC` | 1 | 1 |
| `synth.analog=SUPER` | 47 | 57 |
| `synth.analog=SYNC` | 16 | 18 |
| `synth.wave=BELLS/TINES` | 3 | 3 |
| `synth.wave=DIGITAL` | 6 | 8 |
| `synth.wave=KEYS` | 2 | 2 |
| `synth.wave=ORGAN` | 1 | 1 |
| `synth.sample=loaded` | 119 | 215 |
| `synth.sample.kernel=0` | 115 | 208 |
| `synth.sample.kernel=1` | 119 | 215 |
| `synth.sample.kernel=2` | 119 | 215 |
| `synth.sample.kernel=3` | 119 | 215 |
| `synth.sample.kernel=4` | 119 | 215 |
| `synth.sample.kernel=5` | 119 | 215 |
| `synth.filter=BandPass` | 18 | 24 |
| `synth.filter=HighPass` | 11 | 14 |
| `synth.filter=Ladder24` | 52 | 69 |
| `synth.filter=Lp12` | 132 | 218 |
| `synth.filter=Lp24` | 88 | 119 |
| `synth.unison=1` | 51 | 70 |
| `synth.unison=2` | 47 | 57 |
| `synth.unison=3` | 72 | 91 |
| `synth.lfo_dest=FilterCutoff` | 38 | 52 |
| `synth.lfo_dest=Off` | 226 | 425 |
| `synth.lfo_dest=OscCtrl` | 50 | 66 |
| `synth.lfo_dest=OscPitch` | 33 | 38 |
| `synth.arp` (enabled) | 32 | 49 |
| `synth.mono` (enabled) | 34 | 48 |

SAMPLE layer kernels (384 programs, `96000` frames): 215 of 215 SAMPLES layers (119 programs) load their `.nsmp4`; over the key spans of their zones the firmware key rule ([synth-sample-kernel-rule](synth-sample-kernel-rule.md)) reaches kernels 1, 2, 3, 4, 5 in every loaded layer and kernel 0 in 208 layers of 115 programs. The ASSUMED ratio rule never picks kernels 0 and 1, so it differs from the firmware in every SAMPLES program that reaches them. The 14 WAVE (Wavetable) layers in 12 programs (BELLS/TINES 3, DIGITAL 8, KEYS 2, ORGAN 1) stay unmodelled.

## 6. Effects

| Path | Programs | Occurrences |
|---|---|---|
| `fx.mod1=A-PAN` | 50 | 70 |
| `fx.mod1=A-WAH` | 6 | 7 |
| `fx.mod1=PUMP` | 9 | 19 |
| `fx.mod1=RM` | 3 | 3 |
| `fx.mod1=TREM` | 21 | 23 |
| `fx.mod1=WAH` | 2 | 2 |
| `fx.mod2=CHOR` | 93 | 155 |
| `fx.mod2=ENS` | 3 | 5 |
| `fx.mod2=FLANG` | 10 | 15 |
| `fx.mod2=PHAS` | 12 | 16 |
| `fx.mod2=SPIN` | 11 | 14 |
| `fx.mod2=VIBE` | 45 | 55 |
| `fx.amp=HP24` | 8 | 13 |
| `fx.amp=JC` | 1 | 1 |
| `fx.amp=JC II` | 3 | 3 |
| `fx.amp=LP24` | 19 | 26 |
| `fx.amp=SMALL` | 12 | 16 |
| `fx.amp=TO ROTARY` | 2 | 2 |
| `fx.amp=TWIN` | 4 | 4 |
| `fx.amp=TWIN II` | 19 | 20 |
| `fx.amp=VERY SMALL` | 5 | 6 |
| `fx.amp=off` | 214 | 389 |
| `fx.delay.mode=0` | 42 | 68 |
| `fx.delay.mode=1` | 74 | 112 |
| `fx.delay.mode=2` | 32 | 48 |
| `fx.delay.mode=3` | 17 | 24 |
| `fx.delay_fb=CHOR` | 22 | 28 |
| `fx.delay_fb=ENS` | 5 | 10 |
| `fx.delay_fb=FLAM` | 11 | 14 |
| `fx.delay_fb=SPACE` | 22 | 28 |
| `fx.delay_fb=VIBE` | 11 | 17 |
| `fx.reverb=BOOTH` | 195 | 360 |
| `fx.reverb=ROOM` | 150 | 325 |
| `fx.reverb=SPRING` | 57 | 71 |
| `fx.reverb.type=0` | 40 | 47 |
| `fx.reverb.type=1` | 17 | 24 |
| `fx.reverb.type=2` | 39 | 57 |
| `fx.reverb.type=3` | 160 | 303 |
| `fx.reverb.type=4` | 136 | 269 |
| `fx.reverb.type=5` | 28 | 56 |
| `fx.amp` (enabled) | 261 | 480 |
| `fx.comp` (enabled) | 117 | 204 |
| `fx.delay` (enabled) | 155 | 252 |
| `fx.eq` (enabled) | 265 | 503 |
| `fx.mod1` (enabled) | 90 | 124 |
| `fx.mod2` (enabled) | 158 | 260 |
| `fx.reverb` (enabled) | 358 | 756 |

## 7. Verification status of the paths in use

Uses count a path once per layer or enabled FX unit that needs it; one layer or unit can appear in several rows (for example `fx.amp` and `fx.amp=LP24`), so the totals below are classified path uses, not distinct layers. `default` is the engine default, `firmware-all` is `--firmware-all` (routing aside).

| Path | Programs | Uses | Default | Firmware-all | Basis |
|---|---|---|---|---|---|
| `fx.amp` | 261 | 480 | verified | verified | effects-and-routing: firmware drive stage and cabinet are the default amp path (words VERIFIED, voicing ASSUMED) |
| `fx.amp=HP24` | 8 | 13 | assumed | assumed | effects-and-routing: topology ASSUMED |
| `fx.amp=LP24` | 19 | 26 | assumed | assumed | effects-and-routing: topology ASSUMED |
| `fx.amp=TO ROTARY` | 2 | 2 | unmodelled | unmodelled | effects-and-routing: To Rotary is a pass-through |
| `fx.comp` | 117 | 204 | assumed | verified | effects-and-routing: firmware compressor routine VERIFIED, parameters and bus position ASSUMED |
| `fx.delay` | 155 | 252 | assumed | verified | effects-and-routing: firmware time/mix words VERIFIED (firmware_fx.delay_laws), loop is the engine's |
| `fx.delay_fb` | 68 | 97 | assumed | assumed | effects-and-routing: feedback effects approximated |
| `fx.eq` | 265 | 503 | assumed | assumed | effects-and-routing: RBJ biquads; gain step and Q ASSUMED |
| `fx.mod1` | 90 | 124 | assumed | assumed | effects-and-routing: Mod 1 types are engine models, constants ASSUMED |
| `fx.mod2` | 158 | 260 | assumed | assumed | effects-and-routing: Mod 2 types are engine models, constants ASSUMED |
| `fx.mod2=ENS` | 3 | 5 | assumed | assumed | effects-and-routing: Ensemble approximated (DSP not in ARM image) |
| `fx.mod2=SPIN` | 11 | 14 | assumed | assumed | effects-and-routing: Spin approximated (DSP not in ARM image) |
| `fx.reverb` | 358 | 756 | assumed | assumed | effects-and-routing: reverb network is the engine FDN; wrap routine not wired; mix law verified under firmware-all |
| `organ.drawbars` | 44 | 60 | assumed | assumed | organ-engine: drawbar scaling per model, Legacy for non-Pipe |
| `organ.model=B3` | 27 | 36 | assumed | assumed | organ-model-paths: B3 tone source not located; Legacy wheels; firmware-all forces the Pipe bank (attribution disproved) |
| `organ.model=B3 Bass` | 4 | 4 | assumed | assumed | organ-model-paths: raw model 5 decodes to B3 plus the bass flag (was folded into Pipe2); bass processor 0x122296 not bit-exact (OQ18) |
| `organ.model=Farfisa` | 2 | 3 | assumed | assumed | organ-model-paths: Farfisa tone source not located; Legacy wheels |
| `organ.model=Pipe1` | 3 | 4 | verified | verified | organ-model-paths: 92-wheel bank VERIFIED, role for the pipes ASSUMED |
| `organ.model=Pipe2` | 4 | 5 | verified | verified | organ-model-paths: 92-wheel bank VERIFIED, role for the pipes ASSUMED |
| `organ.model=Vox` | 7 | 8 | assumed | assumed | organ-model-paths: Vox bank is VERIFIED but opt-in (`--tone-source vox`), not in firmware-all |
| `organ.percussion` | 6 | 8 | assumed | assumed | organ-engine: percussion ASSUMED |
| `organ.rotary` | 24 | 35 | assumed | assumed | organ-engine: Rotary 122 model ASSUMED; Leslie routine not wired into the program path |
| `organ.vibrato` | 21 | 25 | assumed | assumed | organ-engine: scanner vibrato/chorus ASSUMED (B3 tail opt-in) |
| `piano.extra_voice=played` | 250 | 276 | verified | verified | piano-extra-voices: classes 1,2,3,5,6 extra voice rule and gain VERIFIED, played by default since D23 (muted only under --piano-slew-fw, where the SMP mutes it) |
| `piano.sample=loaded` | 272 | 311 | verified | verified | piano-engine: .npno stream decode and firmware level law (D23 default) |
| `routing.split` | 48 | 48 | assumed | verified | morph-and-zones: splits/zones follow the program only with --firmware-routing (zone codes partly verified) |
| `synth.analog=MISC` | 8 | 8 | unmodelled | unmodelled | synth-wave-models: not modelled |
| `synth.analog=MULTI` | 43 | 44 | unmodelled | unmodelled | synth-wave-models: not modelled |
| `synth.analog=PURE` | 61 | 90 | assumed | verified | synth-wave-models: PURE saw/sine/triangle/pulse stages verified on the oracle; only the saw is wired (strip chain, firmware-all) |
| `synth.analog=SHAPE` | 50 | 64 | assumed | assumed | synth-wave-models: Shape Saw/Triangle stages verified, shape word unknown; opt-in `--synth-shape` |
| `synth.analog=SHAPE SINE` | 30 | 32 | unmodelled | unmodelled | synth-wave-models: not modelled |
| `synth.analog=SUB OSC` | 1 | 1 | unmodelled | unmodelled | synth-wave-models: not modelled |
| `synth.analog=SUPER` | 47 | 57 | assumed | assumed | synth-engine: Super Saw EMPIRICAL voice spread, stage not modelled |
| `synth.analog=SYNC` | 16 | 18 | unmodelled | unmodelled | synth-wave-models: not modelled |
| `synth.arp` | 32 | 49 | assumed | assumed | synth-engine: arpeggiator ASSUMED (pattern table not located) |
| `synth.envelopes` | 289 | 581 | assumed | verified | synth-engine: strip chain records VERIFIED bit exact, panel mapping ASSUMED |
| `synth.filter=BandPass` | 18 | 24 | assumed | verified | synth-engine: firmware BP4 stage VERIFIED, type map ASSUMED |
| `synth.filter=HighPass` | 11 | 14 | assumed | verified | synth-engine: firmware HP4 stage VERIFIED, type map ASSUMED |
| `synth.filter=Ladder24` | 52 | 69 | assumed | verified | synth-engine: ZDF filter by default; firmware LP4 stage VERIFIED, type map ASSUMED |
| `synth.filter=Lp12` | 132 | 218 | assumed | verified | synth-engine: ZDF filter by default; firmware LP4 stage VERIFIED, type map ASSUMED |
| `synth.filter=Lp24` | 88 | 119 | assumed | verified | synth-engine: ZDF filter by default; firmware LP4 stage VERIFIED, type map ASSUMED |
| `synth.osc=Analog` | 193 | 314 | assumed | assumed | synth-engine: VA waveforms approximate the strip stages |
| `synth.osc=Fm` | 25 | 38 | assumed | assumed | synth-engine: FM ratio index decoded, wave audio ASSUMED |
| `synth.osc=Sample` | 119 | 215 | verified | verified | synth-engine: .nsmp4 PCM played through the verified 12-tap sinc kernels (kernel choice ASSUMED) |
| `synth.osc=Wavetable` | 12 | 14 | unmodelled | unmodelled | synth-engine: WAVE bank is a synthetic 64-table placeholder |
| `synth.sample.kernel=0` | 115 | 208 | assumed | assumed | synth-sample-kernel-rule: kernel chosen by key, VERIFIED run of the ARM chain and key field (D46, default); zone pitch key and tune of the sample map ASSUMED |
| `synth.sample.kernel=1` | 119 | 215 | assumed | assumed | synth-sample-kernel-rule: kernel chosen by key, VERIFIED run of the ARM chain and key field (D46, default); zone pitch key and tune of the sample map ASSUMED |
| `synth.sample.kernel=2` | 119 | 215 | assumed | assumed | synth-sample-kernel-rule: kernel chosen by key, VERIFIED run of the ARM chain and key field (D46, default); zone pitch key and tune of the sample map ASSUMED |
| `synth.sample.kernel=3` | 119 | 215 | assumed | assumed | synth-sample-kernel-rule: kernel chosen by key, VERIFIED run of the ARM chain and key field (D46, default); zone pitch key and tune of the sample map ASSUMED |
| `synth.sample.kernel=4` | 119 | 215 | assumed | assumed | synth-sample-kernel-rule: kernel chosen by key, VERIFIED run of the ARM chain and key field (D46, default); zone pitch key and tune of the sample map ASSUMED |
| `synth.sample.kernel=5` | 119 | 215 | assumed | assumed | synth-sample-kernel-rule: kernel chosen by key, VERIFIED run of the ARM chain and key field (D46, default); zone pitch key and tune of the sample map ASSUMED |

| Configuration | verified | assumed | unmodelled |
|---|---|---|---|
| default | 1291 (19%) | 5401 (79%) | 119 (2%) |
| firmware-all | 2910 (43%) | 3782 (56%) | 119 (2%) |

Use-weighted: a path used by two layers counts twice. A `verified` row means the algorithm is firmware-verified, not that its parameters or the panel mapping are.

## 8. Render health

- `default`: 384 of 384 rendered without panic; non-finite 0, silent 0, any clipped sample 24, clipping >= 0.1%: 5.
- `firmware`: 384 of 384 rendered without panic; non-finite 0, silent 0, any clipped sample 30, clipping >= 0.1%: 6.

Level difference firmware minus default (RMS, dB) over 384 programs: min -8.95, p10 -1.43, median 0.44, p90 3.51, max 9.61, mean 0.69.

### Outliers

Counts: panic 0, non-finite 0, silent 0, clipping 11, level jump 15.

**Clipping (top 30)**

| entry | mode | peak | clip_fraction |
|---|---|---|---|
| Program/Bank D/Windhoek B3 II.ns4p | firmware | 1.0 | 0.113964 |
| Program/Bank D/Windhoek B3 II.ns4p | default | 1.0 | 0.07913 |
| Program/Bank A/B3 Shade.ns4p | firmware | 1.0 | 0.027682 |
| Program/Bank D/Vox Inna-Gadda.ns4p | firmware | 1.0 | 0.01976 |
| Program/Bank A/B3 Shade.ns4p | default | 1.0 | 0.014484 |
| Program/Bank D/Vox Inna-Gadda.ns4p | default | 1.0 | 0.009443 |
| Program/Bank E/Wah EP Ondre J.ns4p | firmware | 1.0 | 0.005302 |
| Program/Bank B/Q and A.ns4p | firmware | 1.0 | 0.00449 |
| Program/Bank C/Vox Like Me.ns4p | firmware | 1.0 | 0.002406 |
| Program/Bank C/TapePiano AT.ns4p | default | 1.0 | 0.001411 |
| Program/Bank C/Vox Like Me.ns4p | default | 1.0 | 0.001047 |

**Level jumps (top 30)**

| entry | rms_delta_db |
|---|---|
| Program/Bank D/16th Note Bass.ns4p | 9.61 |
| Program/Bank B/Fan Club Piano.ns4p | 9.22 |
| Program/Bank E/PineScape Synth.ns4p | -8.95 |
| Program/Bank C/Bounce Clav.ns4p | 8.49 |
| Program/Bank E/Snappy D6.ns4p | 8.34 |
| Program/Bank A/Jazz Guitar II.ns4p | 8.22 |
| Program/Bank E/24 Frames J3.ns4p | 8.08 |
| Program/Bank F/Unclocked Pad.ns4p | 7.5 |
| Program/Bank C/LoFi Piano.ns4p | 7.4 |
| Program/Bank F/Resonant Peaks.ns4p | 7.33 |
| Program/Bank E/J3 Split Special.ns4p | 6.66 |
| Program/Bank F/OD Rock Bass.ns4p | 6.46 |
| Program/Bank A/Dual Gates.ns4p | -6.43 |
| Program/Bank E/Spin Wurly II.ns4p | 6.17 |
| Program/Bank B/Roto EP Ondre J.ns4p | 6.0 |

## 9. Implications

Regenerated 2026-11-14 for the D59 synth defaults (per-wave P rows, Ladder24 cutoff factor 0.8, knob laws default since D57, SUPER and SHAPE stages), `96000` frames, both modes in one run (`default` and `--firmware-all --firmware-routing`), against the numbers committed before (same 384 programs, same notes). Derived numbers only.

- Health: panic 0, non-finite 0, silent 0 in both modes, as before. No program newly clips at >= 0.1 %. Programs with any clipped sample: `default` 37 to 36 (Fat Wheel no longer clips), `firmware` 40 to 36 (Bounce Clav, Ice Pad, Cinema Rise, Fat Wheel, Moving Chords no longer clip; Calling Whl newly clips, below 0.1 %). Clipping >= 0.1 %: `default` 9 to 9, `firmware` 9 to 8 (Cinema Rise drops out).
- Level against the previous numbers (RMS change of the same program, same mode): median 0.0 dB, 55 programs move by more than 1 dB and 23 by more than 6 dB in `default` (50 and 23 in `firmware`). Largest rises in `default`: Scream Lead AT +13.3 dB, 80s Sci-Fi +9.8, Statement Syn Bs +9.7, Warm Syn Brass, Classic Syn Bass, Paisley Synth, Dyn Octave Pad and Funk Hit Split +9.1 to +9.2. Largest drops: Glow Chords Whl -9.3 dB (-12.1 in `firmware`), Sine Rain Pad -7.0, ScissorSplit Whl -6.3, Trumpet Synth and Syn Flute -6.0, Moving Chords -5.8.
- Firmware minus default level (section 8): minimum -7.54 to -9.02 dB, maximum unchanged at +18.39 (Farfisa II, an organ program). Resonant Peaks (+6.66) leaves the jump list; PineScape Synth (-9.02) and 24 Frames J3 (+7.32) enter it. The classification totals (section 7) are unchanged: the status table is a reading of the specs, not of the render.
- The level shifts are model changes validated only by the 15 hardware clips ([synth-validation](synth-validation.md)); no factory program has a hardware level reference, so they stay open under TD-11.

## 10. Follow-ups

- Hardware level references for the synth programs with the largest shifts (Scream Lead AT, 80s Sci-Fi, Glow Chords Whl) to confirm or refit the per-wave P rows (TD-11).

Data: [data/program-sweep.json](data/program-sweep.json). Reproduce: `cargo build --release -p nord-tools --bin program_sweep` then `uv run python tools/program_sweep.py --ns4b ".reference/<backup>.ns4b"`.
