# Piano Engine & Sympathetic String Resonance Specification (`DSPF`)

## 1. Scope

Specifies the 2-layer (`Piano A`, `Piano B`), 120-voice polyphonic Piano Engine, Timbre shaping, Dynamic Pedal Noise, and real-time Modal Sympathetic String Resonance of the Nord Stage 4.

Implemented in [`crates/nord-core/src/piano.rs`](../../crates/nord-core/src/piano.rs).

---

## 2. Piano Categories & Voice Architecture

The Stage 4 organizes piano banks into 6 acoustic and electro-mechanical categories:
1. **`Grand`** — Concert & studio grand pianos (full string resonance + pedal noise).
2. **`Upright`** — Vertical acoustic pianos with wider stereo soundboard bloom.
3. **`Electric`** — Tine (Rhodes Mk I/II/V) and Reed (Wurlitzer 200A) electric pianos with Dyno preamps.
4. **`Clav`** — Hohner Clavinet D6 with pickup & mute filter tabs.
5. **`Digital`** — FM DX7 / MKS-20 / CP-80 layer pianos.
6. **`Misc`** — Harpsichord, Marimba, Vibraphone, Cimbalom.

### Velocity Response Curves (`KBD TOUCH`)

Legacy path (sounds without a firmware level model, `VelocityTouch::map_velocity`, ASSUMED powers) maps MIDI velocity $v \in [1, 127]$ to $v_{\text{norm}} \in (0, 1]$:
- **`Light`:** $\left(\dfrac{v}{127}\right)^{0.65}$
- **`Medium` (Default):** $\left(\dfrac{v}{127}\right)^{1.00}$ (linear response)
- **`Heavy`:** $\left(\dfrac{v}{127}\right)^{1.55}$

Firmware curves (default since D165: `PianoLayer::firmware_touch`; `--legacy-piano-touch` opts out; `--firmware-piano-touch` stays accepted; GUI Firmware bar "Touch curves", D88, [piano-touch-curves](../research/piano-touch-curves.md)): the program Touch is edit id `0x159` (0 heavy, 1 medium, 2 light, default 1; program file bits `1991..1992` of layer A, `+96` of layer B). The handler `0xC011AA24` stores `curveId = 10 / 11 / 13` in the piano layer object, and the voice init reads the velocity position `T = byte[0xC0532664 + (curveId << 10) + r8(v)]` of every note (including MIDI notes). `T` selects the stroke records and gives `X = clamp(((128 - 0x13) T) >> 7, 0, 249)` of the level law below. `piano/touch_curve.rs` holds `T(v)` for the three curves (VERIFIED, 27 of 27 against the ARM run, fingerprint pinned by a unit test). At velocity 64 `T` is 88 / 78 / 68 (heavy / medium / light), i.e. the light curve is 1.8 dB louder and the heavy one 1.6 dB quieter than medium; at velocity 10 the spread is +2.6 / -2.4 dB, at 120 +0.2 / -1.0 dB. Sounds with a level model play the layer's curve (`NpnoLevel::voices_touch`); other sounds multiply the Medium response by `2^(-2 (X - X_medium) / 60)` (`TouchCurve::gain_vs_medium`). Off (`--legacy-piano-touch`): `.npno` notes play the Medium curve whatever the setting, other sounds the powers above. The decoder (`cbin/piano_preset.rs`) reads the Touch at layer base + 63 (D99; factory programs decode 156 heavy / 208 medium / 20 light) and `PianoLayer::touch` now carries it. With `firmware_touch` off, only the sounds without a firmware level model feel it (the ASSUMED powers `VelocityTouch::map_velocity`); `.npno` notes stay Medium. The program file bits equal the firmware recall for all 384 factory programs, both layers (D165, `tools/arm_program_touch_bits.py`, VERIFIED). Default on with owner approval (D165) although no clip exists for the Light and Heavy curves and the fallback sounds scale the Medium response (ASSUMED; layer A: level model in 83 of 83 Heavy or Light programs in use, layer B unchecked). The plugin `Params::sync_to_engine` (`nord-plugin/src/params.rs`) writes the global `piano_touch` parameter (default Medium) to both layers, so the per-program Touch reaches the standalone and the GUI only. The System menu MIDI velocity curve (`Edt_Sys_kMIDIVelCurve`) and the Organ / Synth sections are not part of this model (the Organ and Synth have no Touch id).

### Downstream level after the voice (`piano/downstream.rs`)

Voice output stage (VERIFIED, `smp_voice.rs`): `bus += (y << 17) * gain >> 31`, so a bus word read as a fraction of Q31 full scale is `(y / 8192) * 0.5 * gain` (`FIRMWARE_SAMPLE_SCALE`). The engine sample path multiplies `y / 8192` by `LEGACY_SAMPLE_SCALE = 0.55` (ASSUMED, legacy); `PianoLayer::firmware_bus_scale` (default `false`, `ns4 render-patch --firmware-piano-bus`) uses 0.5 (-0.83 dB). The receiver's full-scale convention is ASSUMED. After the voice the engine adds no level at default settings (layer level 1.0, master compressor amount 0, FX off, timbre `Off`, dyn comp 0, unison 0; string resonance < 1e-5 of RMS), measured in [piano-downstream-level](../research/piano-downstream-level.md) (D24). The video comparison level depends on the replay velocity (0.37 dB per step), see that note.

Layer level ([piano-bus-convention](../research/piano-bus-convention.md), D29): the firmware handler `0xC011A7A4` stores `(v / 127)^2` (VERIFIED, run live, `tools/arm_layer_level.py`); `PianoLayer::firmware_layer_law` (`--firmware-piano-layer-law`, default `false`) applies it in `engine/mix.rs`, the default fader stays linear because the word-to-voice-gain path is not traced. The SMP post-stage ring `0xB2000` is the transmit buffer of the SMP serial port (32-bit float TDM words, `bus * 2^-29`); its receiver is not decoded.

SMP post stage ([piano-bus-convention](../research/piano-bus-convention.md) s2b, D30): run live, unity: `ring = bus * 2^-29` once the ARM gate words are open (VERIFIED, `tools/oracle_scenarios/smp_post_stage.scn`); the receiver is the FX image ([piano-receiver-scale](../research/piano-receiver-scale.md), D39; master level word `(volume / 127)^2 * taper` = 1.0 at the defaults, D48): SPORT3B ten-word frames copied without a multiply, unity master IIR, master gain `DM(0xB651A) = 0.1866` and the DAC word `fix(clip(2 x, 0.999)) * 2^31` (VERIFIED, run), so a bus word reads `bus * 2^-29 * 0.3732` of `2^31` (`bus_word_dac_fraction`, `receiver_path_sample_scale` = 0.7464 per `y / 8192`, 3.5 dB over `FIRMWARE_SAMPLE_SCALE`); the master level word `DM(0xB6518)` (0 in the image, writer not found) is ASSUMED 1.0, so `firmware_bus_scale` stays opt-in.

Opt-in `--firmware-piano-chain` (`PianoLayer::receiver_chain_scale`): the sample path takes the verified FX receiver path `4 G = 0.7464` per `y / 8192` (`downstream::receiver_path_sample_scale`), the engine unit read as the DAC fraction (ASSUMED); on the Velvet Grand clip it lands +10 to +12 dB over the hardware RMS, so the default stays 0.55 ([piano-receiver-scale](../research/piano-receiver-scale.md) s6, D49).

Felt Upright (`A:13`) against the Part A video ([piano-felt-upright-soft](../research/piano-felt-upright-soft.md), D76): the shape matches at velocity 100, the level is +11.0 dB (+8.4 dB at 88, Pipe yardstick +7.3 dB); the on-screen keybed therefore plays velocity 88 (EMPIRICAL), not 100. Receiver and replay ([piano-note-velocity](../research/piano-note-velocity.md)): the first reader of the bus (SMP post stage, pc `0x1C0918`) converts a bus word with `FLOAT BY -29` (`word * 2^-29`, EMPIRICAL); the rest of the path to the DAC is not decoded, so the `2^31` convention stays ASSUMED and the 0.55 constant stays the default. `ns4 render-patch --events` accepts a per-event `velocity` (1..127) that overrides `--velocity`. The Velvet Grand clip matches a uniform replay velocity of about 88 (spectrum within 2.7 dB per band, brilliance +0.5 dB, RMS +7.3 dB = the Pipe yardstick offset); per-note velocities cannot be recovered from the recording. The piano layer level (`0xB7` / `0xB8`) follows `(v / 127)^2` in the firmware (handler `0xC011A7A4`, VERIFIED from the listing, not run); the engine applies it linearly (the two agree at the default 127). With transcribed per-note velocities and pedal ([piano-transcription](../research/piano-transcription.md), D79) the default is -1.6 to -3.8 dB RMS of the hardware on four clips: the video notes are played soft (Felt Upright median velocity about 43).

### Firmware velocity -> level law (opt-in, `piano/velocity_level.rs`)

VERIFIED against the real firmware (OS v1.66, `docs/research/piano-sound-load.md` sections 6 and 7; Clavinet D6, Bright Grand, Amber Upright, note 60, 79 velocities): the piano class level method `0xC00A1EB4` stores the Q23 voice amplitude

$$n = \mathrm{rec}[\mathtt{0x5E}] + 2X + 60 - (b_1 + b_2 + b_3), \qquad A = G[n mod 60] \gg \lfloor n / 60 
floor,\quad G[k] = \mathrm{round}(2^{23} \cdot 2^{-k/60})$$

(`piano/level_q23.rs`; 60 steps per octave, 0.1003 dB per step). `X` is the velocity attenuation `clamp(d + ((128 - c) T) >> 7, 0, 249)`, `T` a byte of the firmware velocity curve for the internal velocity (`0..=1023`), `(c, d)` = `(0x13, 0)` for the zone used; it falls from 170 (velocity 1) to 0 (velocity 121 and above) and was identical for the three sounds swept. `rec[0x5E]` is a field of the selected stroke record (depends on the root key and the velocity layer; file specific), `b1 + b2 + b3` are three signed per-key bytes of the sound object (40 / 49 / 54 at note 60 for the three sounds; they vary with the note). The module provides the `X` table (VERIFIED at 79 velocities, EMPIRICAL interpolation between), the stroke step function (EMPIRICAL between swept velocities) and the index formula; it is **not wired into the default render** (`render_hash` unchanged). `rec[0x5E]`, the stroke choice and the per-key bytes are read from the `.npno` by `piano/npno_level.rs` (`NpnoSound::firmware_level`); `PianoLayer::firmware_velocity_level` (default `true`, D23) makes a note-on of an `.npno` sound start the firmware's voices (`NpnoLevel::voices`: main voice, plus an extra first voice on classes 1, 2, 3, 5, 6) with the gain `level_q23 / 2^23` each (`piano/extra_voice.rs`); the touch curve remains for SFZ/WAV, synthetic sounds and damper-down notes. Open: the downstream level calibration against the hardware recording and the SMP slew times.

---

## 3. Key Release

Releasing a key starts a 120 ms damping envelope on its voice. With the damper pedal down, the note remains sustained until pedal release, which starts the same damping phase. Note Off does not deactivate a voice immediately.

## 4. Timbre EQ Presets (`PIANO TIMBRE`)

Each Piano layer offers selectable spectral contours modeled via 2nd-order biquad shelving/peaking filters:
- **`Off`:** Flat response ($0\text{ dB}$).
- **`Soft`:** $-6\text{ dB}$ high-shelf above $1.8\text{ kHz}$ for warm ballad voicings.
- **`Mid`:** $+3.5\text{ dB}$ presence peak at $1.2\text{ kHz}$ ($Q = 0.9$) to cut through a live band.
- **`Bright`:** $+4.5\text{ dB}$ high-shelf above $2.5\text{ kHz}$.
- **Electric type** (`PianoLayer::category == Electric`, manual p.26): `Soft`, `Mid`, `Bright` are tailored. The bell/presence band moves to the upper mid-range (ASSUMED 2.2 kHz); `Soft` cuts treble and bell (`-0.55 high - 0.30 bell`), `Mid` boosts the bell (`+0.60 bell`), `Bright` boosts bell and treble (`+0.50 bell + 0.65 high`). Gains are ASSUMED (manual wording only); confirm with a hardware sweep of the TIMBRE steps on an EP model. All other types keep the acoustic set above bit-identically. `Dyno1` / `Dyno2` exist for Electric only (the panel cycles Off, Soft, Mid, Bright for the other types). The seven Clav pick-up EQ combinations (p.26) are UI-only; the engine has no Clav timbre yet.
- **`Dyno1` / `Dyno2`** (Electric Pianos): Classic Dyno-My-Piano bell boost ($+6\text{ dB}$ @ $3.8\text{ kHz}$, $+3\text{ dB}$ @ $120\text{ Hz}$, $-4\text{ dB}$ notch @ $800\text{ Hz}$).
- **Firmware timbre gain (opt-in `--firmware-piano-timbre`, D209, `piano/timbre_gain.rs`, [piano-timbre-gain](../research/piano-timbre-gain.md)):** the firmware does not filter for `kEq`; it stores a timbre mode and the voice-frame builder attenuates each new note by a velocity and key dependent gain (0.1003 dB steps, VERIFIED 108 of 108 ARM words). With the flag the gain replaces the contours above on the layers whose mode has tables (generic `kEq & 3` = 1..3 gives modes `0x10..0x12`; Electric classes `0x13..0x17`; Clav class modes sit below `0x10` and are unmodelled). ASSUMED: played key without octave shift; sounding voices keep their start gain. Default off.

---

## 4. Advanced String Resonance (Modal Sympathetic DSP)

When `String Resonance` is active on an Acoustic `Grand` or `Upright` model:
1. **Damper Pedal Down (`CC64 >= 64`):** A bank of 12 tuned modal harmonic resonators (covering the fundamental and low overtones of the soundboard harp) is excited by the audio output of struck notes ($g_{\text{res}} \approx -21\text{ dB}$, decay $T_{60} \approx 4.5\text{ s}$) and summed with balanced alternating stereo panning (`0.72 / 0.28`, avoiding anti-phase cancellation).
2. **Undamped Held Notes (Pedal Up):** Any key held silently (or after note decay) keeps its pitch-class (`midi_note % 12`) resonator undamped across octaves; striking a note excites sympathetic ringing in held strings without artificial high-frequency ringing.

Each resonator is implemented as a 2-pole high-$Q$ bandpass/comb resonator with frequency-dependent damping:

$$
y[n] = (1 - r^2) \sin(\omega_0) \cdot x[n] + 2r\cos(\omega_0) \cdot y[n-1] - r^2 \cdot y[n-2], \qquad r = \exp\left(-\frac{3 \ln 10}{T_{60} f_s}\right)
$$

---

## 5. Dynamic Pedal Noise (`Piano Pedal` in Firmware) & Unison / Dyn Comp

When `Pedal Noise` is enabled and `CC64`, `CC66`, or `CC67` transitions:
- **Damper Lift (`CC64 >= 64`):** Velocity of pedal depression scales a low-passed mechanical felt thump ($f_c = 140\text{ Hz}$) plus an airy felt-release brush transient (`pedal_brush_env`) exciting the sympathetic string resonator bank (using filtered impulse transients without synthetic sine chirps).
- **Damper Drop (`CC64 < 64`):** Generates a deeper damped felt thud ($f_c = 95\text{ Hz}$).
- **Sostenuto (`CC66`):** Captures notes currently active at pedal-down (`sostenuto_held`) until pedal release and triggers a lighter mechanical rod transient ($f_c = 120\text{ Hz}$).
- **Una Corda / Soft (`CC67`):** Softens hammer strike excitation ($0.76\times$ gain, $0.90\times$ post-shaping) and triggers rail-shift noise ($f_c = 160\text{ Hz}$).
- **Piano `UNISON` (`1, 2, 3`) & `DYN COMP` (`1, 2, 3`):** `unison` adds symmetric stereo hammer/string detune (`3.5`, `7.0`, `11.5` cents) across active voices; `dyn_comp` compresses strike velocity curves and applies fast output dynamic leveling.

---

## 6. Damper Release Envelopes & Soft Release (`SOFT REL`)

Upon receiving MIDI `NoteOff` or Damper Pedal Release (`CC64 < 64`), the vibrating string is not instantly silenced to zero; instead, an authentic felt damper contact decay is applied:

1. **Standard Key Release:**
   Exponential envelope decay with time constant $\tau = 80\text{ ms}$ ($v_{\text{env}}[n] = v_{\text{env}}[n-1] \cdot \exp(-1 / (0.080 \cdot f_s))$). This completely eliminates high-frequency step discontinuities and audio clicks.
2. **Soft Release (`SOFT REL`):**
   When `soft_release` is active on the front panel, the damper descends with a gentler time constant $\tau = 180\text{ ms}$, modeling lighter felt contact for expressive legato phrasing and ambient tail preservation.
3. **Voice Deactivation Invariant:**
   A voice remains in `releasing` state and active in the polyphonic voice matrix until $v_{\text{env}} < 10^{-4}$ ($-80\text{ dB}$), at which point it is safely marked inactive (`v.active = false`) without memory allocations or locking.

---

## 7. Multisample Stroke Playback & Pitch Transposition

Implemented in [`crates/nord-core/src/piano.rs`](../../crates/nord-core/src/piano.rs) (`NpnoSound`, `LoadedPianoStroke`).

When a pre-decoded `.npno` or open SFZ/WAV (`NpnoSound::from_sfz_str` / `from_sfz_path`, `parse_wav_mono` 8/16/24/32-bit mono/stereo) instrument is loaded via `PianoLayer::set_sound`:
1. **Zone, Velocity, Damper & Release Lookup (`NpnoSound::select_stroke_with_damper`):**
   Multi-velocity `.npno` strokes and SFZ `<region>` zones (`lovel..=hivel`, `lokey..=hikey`, `trigger=release`) are deduplicated in RAM (`Arc<[f32]>`). In `.npno` banks, byte `+0x0D` (`vel_low`) encodes descending attenuation (`0 = fff` loudest/brightest down to `max_att = ppp` softest/warmest); zones are sorted descending and partitioned across `0..=127`. When `damper_pedal_down` is active, `.npno` `kind = 1` (`NPNO_STROKE_DAMPER`) sympathetic harp strokes are selected ahead of dry `kind = 0` strokes. Held sampled voices preserve their natural recorded decay envelope (`v.env = 1.0`) until note-off or sample end, while key-off events spawn matching release (`kind = 2` / `trigger=release`) samples without heap allocation. For `.npno` files, `key_map[n]` and `tuning_cents[n]` resolve root mapping and stretch tuning directly.
2. **Pitch Transposition Ratio:**
   The fractional playback speed increment per engine sample frame is calculated as:
   $$
   r = 2^{\frac{(n - n_{\text{root}}) + \frac{c}{100}}{12}}, \qquad \text{sample\_step} = r \cdot \frac{f_{\text{s, stroke}}}{f_{\text{s, engine}}}
   $$
3. **SMP 6-Tap Q31 Sample Interpolation (`SampleInterp::SmpQ31Hw`, default; `SmpQ31` is the f32 form):**
   All fractional sample lookups (including `35 kHz -> 48 kHz` `.npno` hardware sample-rate conversion and semitone transposition) use the kernel the SMP DSP image applies (`crates/nord-core/src/piano/q31_kernel.rs`; see "SMP sample interpolation" below). `SampleInterp::Cubic` selects the earlier zero-allocation 4-point Catmull-Rom spline over `[p_{k-1}, p_k, p_{k+1}, p_{k+2}]` (`PianoLayer::sample_interp`).
4. **Resilient Fallback:**
   If no sample is loaded, or if the note is outside the mapped multisample zones, the engine smoothly falls back to the mathematical harmonic oscillator bank.

---

## 8. Polyphonic Voice Stealing Priority

When note-on events exceed the pre-allocated voice capacity (`MAX_PIANO_VOICES = 64` per layer):
1. **Same-Note Retrigger:** If sustaining voices are already sounding the exact same MIDI note, they are transitioned into rapid felt damping while a free slot is allocated via `allocate_voice_slot(None)` so the previous strike decays cleanly without zero-sample hard clicks.
2. **Free Voice:** The first inactive voice (`!v.active`) is selected.
3. **Priority Stealing:** If all voices are occupied, the allocator computes a stealing score:
   $$
   \text{score} = \begin{cases} v_{\text{env}} \cdot 0.1 & \text{if } v_{\text{releasing}} \\ v_{\text{env}} & \text{otherwise} \end{cases}
   $$
   The voice with the lowest score (decaying released notes first, then quietest sustaining notes) is recycled, preserving loud bass roots and active top melody notes without audible voice drops.

---

## 9. SMP sample interpolation (hardware parity)

Source: [smp-kernel-fit](../research/smp-kernel-fit.md) (VERIFIED, `tools/ns4_smp_kernel_fit.py`, 0 mismatches over the 1,536 words at SMP `DM(0xE3400)`). Code: `piano/q31_kernel.rs` (kernel and interpolator), `piano/smp_unpack.rs` (unpacker and predictor model).

| Item | Definition |
|---|---|
| Position | `pos = n + frac`; `phase p = floor(frac * 256)` (hardware: `frac >> 24`, truncation) |
| Taps | tap `t = 0..5` weighs sample `n - 2 + t`; samples past either end repeat the edge sample (ASSUMED) |
| Offset | `d = (2 - t) + p / 256` |
| Kernel | `u(d) = sinc(d) * (0.54 + 0.46 cos(pi d / 3))`; `h[t][p] = u + (1 - sum_t u) / 6` |
| Word | `min(round(h * 2^23), 2^23 - 1) << 8` (Q31, low byte zero; the peak saturates to `0x7FFFFF00`) |
| Output | `y = sum_t h[t][p] * x[n - 2 + t]`; the integer model `mac_q31` rounds the upper 32 bits of `2 * sum a w` to nearest (ties up, ASSUMED) |
| Properties | `h[t][p] = h[5-t][256-p]` exactly; phase 0 is a pass-through; per-phase tap sum 1 (+- 2 LSB of 23 bits) |

The table is built once (`SmpKernel::shared()`, called in `PianoLayer::new`); `render_frame` only reads a `&'static` table (6 MACs per channel, no allocation, no lock).

`PianoLayer::sample_interp` selects the path: `SmpQ31Hw` (default, hardware parity: the integer MAC `SmpKernel::interpolate_hw` for integer `.npno` strokes, `LoadedPianoStroke::integer_pcm`; float WAV / SFZ strokes use the f32 kernel), `SmpQ31` (f32 kernel for all strokes) or `Cubic` (Catmull-Rom). Measured worst-case error of interpolating a sine at fractional positions (kernel vs Catmull-Rom, Python model in the research note): 0.05 fs -71 / -66 dB, 0.10 fs -55 / -47 dB, 0.15 fs -50 / -35 dB, 0.20 fs -55 / -27 dB, 0.25 fs -36 / -19 dB, 0.30 fs -22 / -14 dB. The unit test `test_sine_tone_error_against_ideal_and_cubic` pins the kernel below the cubic error at 0.2 fs. Like the hardware, the kernel is a fixed interpolator with no anti-alias filtering for upward transposition.

`piano/smp_unpack.rs` models the voice-loop residual unpacker (LSB-first, 16 payload bits per 32-bit word, `FEXT`/`FDEP (SE)`, widths 1..16) and the 4-tap integer predictor; tests prove it equals `cbin::sample` decoding of the same synthetic residuals for widths 1..16 and orders 0..4.

`piano/smp_voice.rs` and `piano/smp_page.rs` model the whole voice call (stream fetch, predictor, history ring, kernel, shift by 17, gain, bus add, position update) and play a `.npno` page; they equal the SMP image on the oracle bit for bit ([smp-voice](../research/smp-voice.md): counter, phase, history, interpolated value and bus, 17 runs on real piano strokes and two committed synthetic goldens). Two differences to the f32 engine path are VERIFIED there: the DSP rounds the six-tap sum to a whole decoded sample (`R2 = MRF (SSFR)`) before scaling, so its output is a multiple of 1/8192 (`SmpKernel::interpolate_hw` reproduces it; the f32 path differs by at most 0.5/8192, 76 dB SNR typical); and the DSP starts from a zero history while the engine repeats the first sample (ASSUMED rule, the ARM preload is unknown). The voice decodes one sample per call, so the per-voice step must stay at or below 1 (ASSUMED consequence for the ARM's multisample choice).

### SMP gain slew and stream edges (`piano/smp_slew.rs`, `piano/ext40.rs`)

Source: [smp-slew](../research/smp-slew.md) (VERIFIED on the oracle, D68). Every second audio interrupt the SMP updates each voice record: ramp `w = max(clip(F-2 + F-1, |F3|), F2)` (stored in `F-2`), x lane `v = max(F11 + F15 * F13 * F17, 0)` (`F15`), y lane `u = max(F12 + F16 * F14 * F18, 0)` (`F16`, and `F13` for the next update; `F14 *= F18`), and the gains `F0 = (fix31(w v) * F19) >> 31`, `F1 = (fix31(w v) * F20) >> 31`, in 40-bit extended float with truncation (`MODE1 = 0x1001CFC`; 32-bit mantissa between operations, 24 bits when stored). `SlewRecord::step` reproduces 11 of 11 random oracle records and 60 of 60 trajectory updates. It is not part of the render path: the ARM-side initial values of ten of its inputs are not decoded (the note-on message list gives `F11 = F12 = 0`, `F14 = 1`, `F16 = 0.9793`), so the engine keeps its own `env` / release.

Wired opt-in (`piano/slew_voice.rs`, `PianoLayer::firmware_slew`, `--piano-slew-fw`, [smp-slew](../research/smp-slew.md) s6, D68): the main voice of an `.npno` note uses the SMP gain `F0 / F1` as its envelope. Note-on record (VERIFIED, 4 sounds): ramp `F-2 = 0.25`, step 0, ceiling 0.5, `F13 = F14 = F16 = F18 = 1`, `F15 = level`, `F17` = rate word, `F19 = F20 = 0x70000000`, so the hold gain is `0.21875 * level` with the decay `1 - d / 2^24` per update (2 to 5 s, rate table EMPIRICAL, interpolated from Velvet Grand); release resets `F11 = F12 = 0`, `F14 = 1`, `F16 = 0x3F7FBB6E` (44 ms time constant). The extra voice has ramp 0 and is not started; release samples and damper-down strokes keep the engine paths. Update clock `43,402.34 / 2` Hz (ASSUMED one interrupt per frame). Default off. The rate word `F17 = 1 - d / 2^24` follows velocity and key (`rate_for(velocity, note)`: bilinear over the Velvet Grand note-on words at velocity 20 / 64 / 88 / 100 / 127 and notes 36 / 60 / 84, EMPIRICAL; the grand sounds differ by at most 15 %, other sounds are ASSUMED to share it). Clip result (D72, [piano-receiver-scale](../research/piano-receiver-scale.md) s7): with `--firmware-piano-chain` the Part A clip reads -2.2 / -3.0 dB RMS (velocity 100 / 88) and the hold and release metrics improve, but a held chord reads 22 dB lower relative to the synth than on the DSoundman clip, so `firmware_slew` and `receiver_chain_scale` stay opt-in.

Key-up sample of a slewed voice (D103, [piano-attack-fw](../research/piano-attack-fw.md)): with `firmware_slew` the release sample takes the firmware record, gain `0.21875 * F15` (`0x3F7FFFFE` up to velocity 100, `0x3F6C1CEA` at 127, linear between ASSUMED) and decay by `F17 = 0x3F7FF38E` (time constant 0.24 s, VERIFIED words); the engine constants (0.30, 1.15 s) stay for the default and for pedal-down voices. The slew has no attack ramp (`F-2 = 0.25` from the first update), so the attack equals the default engine's.

Slew rate word (D110, [piano-attack-oracle](../research/piano-attack-oracle.md)): `F17 = 1 - d / 2^24`, `d = 2 * round(-s * 2^(semitones / 12))`, `s` the `i16` at stroke record byte `0x3C` (VERIFIED 48/48 Felt Upright, 7/7 Velvet Grand; rounding and cents ASSUMED); used by `--piano-slew-fw`.

Stream edges (VERIFIED runs): a counter at or above `F-8` wraps by `F-7` and rotates the history offset `F-24` by the same amount; the slew rate `F17` is `F+6` above `F-8 - F-7` and `F+7` below; a counter at or above `F-9` frees the voice; there is no step clamp (step above 1 skips residuals) and no end test on the 256-word ring. The engine's edge-sample repeat (section 9) is an approximation of loop and stop by position.

### Extra voice and the damper-down voice list (`piano/extra_voice.rs`, `piano/pedal_voices.rs`, D123)

Source: [piano-extra-voices](../research/piano-extra-voices.md). The stroke record byte 4 (`& 3`, the selector `0xC00A2A54` list index, called layer in earlier notes) is the stroke kind: 0 normal, 1 damper-down, 2 key-up (VERIFIED, record dump against the level calls). A note-on of classes 1, 2, 3, 5, 6 starts an extra voice first (default, D23): the damper-down stroke of the root when it has one (grands and uprights: Velvet 22 of 38 roots, Amber Upright 58 of 58), otherwise a second normal stroke at velocity position `T + 20` (EP, Wurlitzer, EGrand, Bright Grand); classes 4, 7, 14, 15, 16 have the main voice only. With the sustain pedal down the firmware runs the same level calls (same records, same gains) as with the pedal up on all 38 factory sounds (VERIFIED, `tools/arm_piano_pedal.py`); the pedal only holds the release. Default engine: pedal-down notes use the legacy damper-stroke path. Opt-in `PianoLayer::firmware_pedal_voices` (`--firmware-piano-pedal-voices`): pedal-down notes start the firmware voices (with the slew when `--piano-slew-fw` is on); the engine's own sustain logic holds and releases them. The firmware key-up record placement under the pedal is not modelled. Census (D202, `piano_voice_census`, [piano-extra-voices](../research/piano-extra-voices.md) s6): on the 38 factory sounds the pedal-up and opt-in pedal-down engine voices equal the firmware list at 6,834 of 6,834 (note, velocity) points; the default pedal-down path equals it on 0 of 4,680 two-voice points.

## 9a. Sample end: sustain loop (D249, VERIFIED) and tail fade (D243, ASSUMED)

Source: [piano-sample-end](../research/piano-sample-end.md). A `.npno` stroke carries a sustain loop: `NpnoStroke::loop_start` = `u32` at stroke record byte `0x24`, loop length = `u32` at `0x28`, `loop_end` = sum (the SMP words `F-8`, `F-7`; `F-9` is `total_frames`), `loop_decay` = `i16` at `0x38`. `sample_loop::wrap` wraps the read position by the loop length at the loop end, so a held note never reaches the stroke end (VERIFIED: voice record replay, 2 strokes, 260,000 passes). Above the loop start the voice envelope decays per frame by `exp(-x * 21,701 / fs)`, `x = 2 * -loop_decay / 2^24` (the firmware rate word `F+6`, `d = 370` = 4.16 dB/s on Astoria Grand; law ASSUMED beyond two strokes); the voice is freed below `1e-4`. Tests: `piano::tests::held_looped_note_outlasts_its_sample`, `looped_note_decays_in_the_loop_and_is_freed_when_silent`, `piano::sample_loop::tests`.

### Tail fade of strokes without loop (D243, ASSUMED)

A non-release voice of a stroke without loop points (SFZ without `loop_start` / `loop_end`) is scaled by `piano/tail_fade.rs::tail_gain`: 1 until the last `0.25 s * stroke rate` (at most a quarter of the stroke), then `0.5 - 0.5 cos(pi * remaining / fade)` down to 0 at the last frame. The voice is freed when the stroke runs out, as before. Key-up strokes are not faded. The length and shape are ASSUMED; a 10 s hardware recording of a held note would confirm them (OQ20). Test: `piano::tests::held_note_fades_out_at_sample_end`, `piano::tail_fade::tests`.

## 10. Candidate: firmware detuned oscillator bank as string resonance (not implemented)

Status: **candidate, ASSUMED, not implemented.** The PNO Core1 routine `0x122620` is a 184-element phase-accumulator wavetable bank: 92 fixed chromatic pitches, each in two copies detuned by about 1.89 cent rms, no per-note state, notes act only through ARM-written weights ([organ-tonewheel-bank](../research/organ-tonewheel-bank.md), algorithm VERIFIED by oracle; the mix stage `0x12268F` is not reproduced). The module chain installs it from the PNO piano state machine and from no ORG/Core2 module id ([pno-module-chain](../research/pno-module-chain.md), branch `research/pno-module-chain`).

| Observation | Fits string resonance |
|---|---|
| Fixed pitch ladder of 92 semitones, always running | sympathetic strings exist for every key regardless of notes held |
| Two copies per pitch, spread about 2 cent | unison-style detuned string pair |
| ARM has `Edt_Sys_kStringResonanceLevel` (`0xC02D57D0`) next to `Edt_Sys_kToneWheelMode` (`0xC02D57F0`) | a level control for such a bank exists |
| Role judgement | superseded by `docs/research/organ-tone-source.md`: about 70 % organ tonewheels, about 20 % string resonance only (ASSUMED) |

The modal resonators of section 4 are the shipped model; this bank would replace them only after the ARM weight writer or a running capture confirms the role. Open items stay in `docs/OPEN-QUESTIONS.md` and TD-04.

## 5. Piano types and models (PIANO SELECT, manual p.24-25)

The TYPE button selects one of six types (Grand, Upright, Electric, Clav, Digital, Misc); the PIANO SELECT dial steps through the models of that type; the TYPE LEDs show the type of the current model; LIST shows `Piano <Type> n/N` and INFO `Type: <Type> n/N`. The model's type is the `bank_index` of its `.npno` CBIN header and its place in the type is `location_index` (VERIFIED, [piano-types](../research/piano-types.md)). The GUI library (`library/piano_catalog.rs`) reads only the 16 header bytes of each `.npno` entry (archive folder, then `Misc`, as fallbacks), lists models in hardware order and publishes `(entry, type)` pairs; TYPE loads the last used (else first) model of the next type that has models. Without a library TYPE only changes the engine type (Electric timbre set, Dyno steps) and the OLED reads `No piano library`. Engine `category` is set when a sound loads (its header type) and by TYPE; loading a program sets it from the preset.

## Decay and attack against the hardware (D86)

`tools/piano_decay.py` measures the T60 of each note's fundamental per octave in a hardware clip and in emulator renders of the same transcribed notes and pedal (EMPIRICAL). Result ([piano-decay-attack](../research/piano-decay-attack.md), `docs/research/data/piano_decay_per_octave.json`): the default engine's median T60 over the hardware's is 1.00 over 9 octave groups of four clips; held library notes fall 40 dB in 3.7 to 5.8 s and then end with the sample. A stroke voice has no engine decay envelope while held; after key-up the engine envelope is 80 ms (hard) or 180 ms (soft release / half pedal) and the release sample decays with 1.15 s (all ASSUMED). The firmware hold decay (time constant 2.1 to 5 s, VERIFIED on Velvet Grand) and the 44 ms note-off decay live only in the opt-in slew voice (`--piano-slew-fw`). Default attack is about 10 ms faster than the hardware; chain + slew 47 to 70 ms against 56 to 64 ms.

## Key-up sample law (D226)

Source: [piano-release-voice](../research/piano-release-voice.md). The key-up stroke (kind 2, order 4 predictor row `(-1, 4, -6, 4)`) decodes bit for bit as on the SMP (VERIFIED run, voice 17 history ring). Firmware record: step of the note, `F15 = 0x3F7FFFFE` (0.922 at velocity 127), `F17 = 0x3F7FF38E`; gain `0.21875 * F15` against the note's `0.21875 * level` (VERIFIED, oracle trajectory test `slew_voice::tests::key_up_record_equals_the_oracle_trajectory`); time constant 0.2426 s. Engine default: amplitude `0.30 * level` (soft release 0.45), decay 1.15 s (ASSUMED). Opt-in `PianoLayer::firmware_key_up` / `--piano-keyup-fw`: amplitude `F15` in note-amplitude units (ratio `F15 / level`, capped at 8 times the note) and decay 0.2426 s on the engine envelope path; no effect with `firmware_slew`. Default `false` (D22); the unit mapping engine amplitude to record gain is ASSUMED.
