# FX types against the FX chip oracle: level, response, routing

Checked: 2026-11-17.

Sources: `tools/fx_vs_oracle.py` (live FX chain, SHARC oracle, `docs/research/data/fx-vs-oracle.csv`, `fx-vs-oracle-mod1-2frame.csv`); `crates/nord-tools/src/bin/fx_engine_vs_fw.rs` (engine against firmware port, `docs/research/data/fx-engine-vs-fw.csv`); `crates/nord-core/src/fx/fw_live.rs` (context export); [fx-oracle-live](fx-oracle-live.md), [fx-type-map](fx-type-map.md), [reverb-fw-vs-hw.md](reverb-fw-vs-hw.md), D125, D133, D145, D156. No firmware, sample or audio content is committed: numbers and paths only.

Question: for every FX type, does the native engine produce the level, frequency response and time response of the firmware FX chip, and where does it not? Only ARM + SHARC firmware execution is used; no recordings.

## 1. Method

Two measurements, chained so that engine against firmware is covered end to end.

| Step | Compares | How | Proves |
|---|---|---|---|
| A. Port against oracle | Rust firmware port (`FirmwareFx`, opt-in D22) against the FX chip routine run live in the SHARC oracle (`sharc-fxlive`, FX Core 1, 4 frames per pass) | `fw_live::live_install` exports the port's context, rings and routine pc; the oracle runs the same routine with the same words on the same stimulus; best lag, fit gain, null depth, level delta against the chain gain | port = firmware |
| B. Engine against port | Native engine path (`LayerFxChain` with every `firmware_fx` flag off) against the port (flag of that family on) | `fx_engine_vs_fw`: same deterministic stimulus, rms, octave band gains, centroid, first echo, T60 | engine vs firmware, as far as step A covers the port |

Stimuli (deterministic, `fx_cases::stimulus`): impulse (0.5), log sine sweep 30 Hz to 16 kHz over 0.5 s (0.25), white noise (seed 7, rms 0.1, 0.5 s), held-note stream (two 6-harmonic notes, 220 and 330 Hz, release 80 ms). Rate 43,402.344 Hz (DSP frame rate, no rate scaling).

Chain gain: the identity stage in the live chain has gain 0.1866 = master gain `DM(0xB651A)`; "level delta" below is measured against it, so 0.00 dB means the routine passes exactly the level of the port.

Status words: `match` (level delta below 0.01 dB and null depth below -70 dB after the fit gain), `differs by X dB`, `not runnable` (reason given).

## 2. Summary

| Family | Port against oracle (A) | Engine against port (B) |
|---|---|---|
| Mod 2 (12 types) | match, all (null -72 dB or better) | differs, 0.9 to 5.9 dB level, up to 17.7 dB octave band spread |
| Mod 1 (12 types) | match, all (needs 4-frame pass, section 6.2) | differs, up to 5.7 dB level; A-WAH / WAH 15 to 17 dB band spread |
| Reverb types 0, 2 to 5, 7 to 11 | match, all | differs: wet level +9 to +17 dB at amount 127, first echo 3 to 4 times later, T60 differs |
| Reverb type 6 (Stage) | not runnable on the port (falls back to the engine) | not runnable |
| Compressor | match | engine 1.7 to 10.9 dB lower than the port (makeup gain, section 6.5) |
| EQ | not run live (tone stack stays 2-frame, section 5) | within 2 dB, band deltas up to 5 dB at the shelf edges |
| Amp drive + cabinet | not runnable (tables in the PNO image) | modes 1 to 6: +4 to +24 dB at drive 5; port silent at drive 10 (section 6.3); modes 7 to 9 equal |
| Delay | not wired (`FwDelay`) | level -2.9 .. +1.1 dB, echo time equal (411 ms) |
| Delay feedback FX (types 2 to 11) | not runnable (no port) | engine only |
| Rotary / Leslie | not runnable: gate `0x1256AD` not installable (D135, D148, D155) | not runnable |

## 3. Port against oracle (live FX chain)

Each row: cases are rate 0 / 64 / 127 and amount 0 / 64 / 127 on impulse, sweep, noise and notes (subset, see `fx-vs-oracle.csv`). Null depth is the residual after the best fit gain.

### 3.1 Mod 2
| Type | kFx2Type | Cases | Max level delta (dB) | Null depth range (dB) | Worst case | Status |
|---|---|---|---|---|---|---|
| PHAS 0 | 0 | 8 | 0.00 | -132 .. -104 | notes r64 a64 | match |
| PHAS 1 | 1 | 8 | 0.00 | -146 .. -102 | noise r64 a127 | match |
| FLANG 0 | 2 | 8 | 0.00 | -103 .. -72 | noise r64 a127 | match |
| FLANG 1 | 3 | 8 | 0.00 | -103 .. -72 | noise r64 a127 | match |
| CHOR 0 | 4 | 8 | 0.00 | -156 .. -120 | noise r64 a127 | match |
| CHOR 1 | 5 | 8 | 0.00 | -105 .. -84 | noise r64 a127 | match |
| VIBE 0 | 6 | 8 | 0.00 | -138 .. -127 | notes r64 a64 | match |
| VIBE 1 | 7 | 8 | 0.00 | -138 .. -127 | notes r64 a64 | match |
| ENS 0 | 8 | 8 | 0.00 | -156 .. -78 | impulse r64 a64 | match |
| ENS 1 | 9 | 8 | 0.00 | -156 .. -78 | impulse r64 a64 | match |
| SPIN 0 | 10 | 8 | 0.00 | -135 .. -95 | noise r64 a127 | match |
| SPIN 1 | 11 | 8 | 0.00 | -134 .. -95 | noise r64 a127 | match |

### 3.2 Mod 1

The port renders Mod 1 types 4 to 11 in 2-frame calls by default; the live pass is 4 frames, so the comparison uses the opt-in `mod1_rest_frames = 4` (section 6.2). With the default 2 frames the same rows null only to -16 .. -26 dB (`fx-vs-oracle-mod1-2frame.csv`).

| Type | kFx1Type | Cases | Max level delta (dB) | Null depth range (dB) | Worst case | Status |
|---|---|---|---|---|---|---|
| A-PAN 0 | 0 | 8 | 0.00 | -548 .. -136 | notes r64 a64 | match |
| A-PAN 1 | 1 | 8 | 0.00 | -542 .. -114 | noise r64 a127 | match |
| TREM 0 | 2 | 8 | 0.00 | -542 .. -119 | noise r64 a127 | match |
| TREM 1 | 3 | 8 | 0.00 | -542 .. -100 | noise r64 a127 | match |
| RM 0 | 4 | 8 | 0.00 | -544 .. -124 | sweep r64 a64 | match |
| RM 1 | 5 | 8 | 0.00 | -544 .. -124 | sweep r64 a64 | match |
| A-WAH 0 | 6 | 8 | 0.00 | -127 .. -102 | impulse r64 a64 | match |
| A-WAH 1 | 7 | 8 | 0.00 | -126 .. -106 | noise r0 a64 | match |
| WAH 0 | 8 | 8 | 0.00 | -124 .. -106 | sweep r64 a64 | match |
| WAH 1 | 9 | 8 | 0.00 | -137 .. -122 | noise r127 a64 | match |
| PUMP 0 | 10 | 8 | 0.00 | -546 .. -140 | noise r64 a127 | match |
| PUMP 1 | 11 | 8 | 0.00 | -548 .. -155 | noise r64 a64 | match |

### 3.3 Reverb

Reverb type 0 (Spring) and 2 to 11 run through the live reverb routine; Stage (`kRvbType` 6) has no firmware path in the port.

| Type | kRvbType | Cases | Max level delta (dB) | Null depth range (dB) | Worst case | Status |
|---|---|---|---|---|---|---|
| Spring (type 0) | 0 | 4 | 0.00 | -137 .. -124 | sweep r64 a64 | match |
| Booth | 2 | 6 | 0.00 | -160 .. -89 | notes r64 a127 | match |
| Booth Predelay | 3 | 6 | 0.00 | -160 .. -88 | notes r64 a127 | match |
| Room | 4 | 6 | 0.00 | -159 .. -92 | noise r64 a127 | match |
| Room Predelay | 5 | 6 | 0.00 | -159 .. -92 | noise r64 a127 | match |
| Stage Chorale | 7 | 6 | 0.00 | -159 .. -90 | noise r64 a127 | match |
| Hall | 8 | 6 | 0.00 | -159 .. -90 | noise r64 a127 | match |
| Hall Chorale | 9 | 6 | 0.00 | -159 .. -90 | noise r64 a127 | match |
| Cathedral | 10 | 6 | 0.00 | -159 .. -89 | noise r64 a127 | match |
| Cathedral Chorale | 11 | 6 | 0.00 | -159 .. -90 | noise r64 a127 | match |

### 3.4 Compressor and identity

| Stage | kCmpAmt | Cases | Max level delta (dB) | Null depth range (dB) | Worst case | Status |
|---|---|---|---|---|---|---|
| Compressor | 0 | 8 | 0.00 | -553 .. -117 | noise r64 a127 | match |
| identity | 0 | 4 | 0.00 | -548 .. -156 | notes r64 a64 | match |

## 4. Engine against port (step B)

Delta is engine minus port, so a positive number means the engine is louder. Band spread is the range of the nine octave band gains (31 Hz to 16 kHz); a flat 0 spread with a constant level delta is a pure gain difference. Wet level is for white noise at 0.1 rms.

### 4.1 Mod 1

| Type | Level engine - port, noise, amt 0 / 64 / 127 (dB) | Held notes, amt 64 (dB) | Octave band spread, amt 64 (dB) | Centroid engine / port, amt 64 (Hz) | Status |
|---|---|---|---|---|---|
| A-PAN 0 | +0.0 / +2.8 / +5.7 | +2.8 | 2.1 | 10633 / 10726 | differs by up to 5.7 dB, bands +2.2 .. +4.3 |
| A-PAN 1 | +0.0 / +1.6 / +2.7 | +1.9 | 3.0 | 10633 / 10623 | differs by up to 2.7 dB, bands +0.6 .. +3.6 |
| TREM 0 | +0.0 / -0.5 / -1.1 | -0.2 | 2.7 | 10621 / 10618 | differs by up to 1.1 dB, bands -1.6 .. +1.1 |
| TREM 1 | +0.0 / -0.8 / -2.2 | -0.5 | 3.2 | 10621 / 10621 | differs by up to 2.2 dB, bands -1.9 .. +1.3 |
| RM 0 | +1.2 / +1.2 / +1.1 | +0.7 | 3.3 | 10723 / 10586 | differs by up to 1.2 dB, bands -1.7 .. +1.6 |
| RM 1 | +1.2 / +1.2 / +1.1 | +0.7 | 3.3 | 10723 / 10586 | differs by up to 1.2 dB, bands -1.7 .. +1.6 |
| A-WAH 0 | +0.2 / -3.6 / -4.4 | +2.8 | 15.2 | 7702 / 4505 | differs by up to 4.4 dB, bands -7.0 .. +8.2 |
| A-WAH 1 | +3.4 / +1.2 / +0.9 | -0.3 | 13.5 | 7702 / 3644 | differs by up to 1.2 dB, bands -5.9 .. +7.6 |
| WAH 0 | +2.8 / +0.1 / -0.3 | +1.3 | 7.4 | 7702 / 7135 | differs by up to 0.3 dB, bands -2.9 .. +4.5 |
| WAH 1 | +4.1 / +2.6 / +2.5 | -0.6 | 16.9 | 7702 / 3422 | differs by up to 2.6 dB, bands -7.9 .. +9.0 |
| PUMP 0 | +0.0 / +1.0 / +2.5 | +1.0 | 1.6 | 10621 / 10670 | differs by up to 2.5 dB, bands +0.3 .. +1.9 |
| PUMP 1 | +0.0 / -2.1 / -4.0 | -2.1 | 1.1 | 10621 / 10667 | differs by up to 4.0 dB, bands -2.9 .. -1.8 |

Level at amount 0 is 0 dB for the tremolo and pan types (dry) and shows the engine's fixed offset for RM, A-WAH, WAH (+0.2 .. +4.1 dB). A-WAH and WAH differ most in response (centroid 7.7 kHz engine against 3.4 .. 7.1 kHz port at amount 64): the engine band-pass sweeps differently.

### 4.2 Mod 2

| Type | Level engine - port, noise, amt 0 / 64 / 127 (dB) | Held notes, amt 64 (dB) | Octave band spread, amt 64 (dB) | Centroid engine / port, amt 64 (Hz) | Status |
|---|---|---|---|---|---|
| PHAS 0 | -3.3 / -2.3 / -1.8 | -0.8 | 17.7 | 11747 / 9026 | differs by up to 2.3 dB, bands -6.4 .. +11.3 |
| PHAS 1 | -0.7 / -2.4 / -5.9 | -2.7 | 6.6 | 11747 / 11120 | differs by up to 5.9 dB, bands -5.6 .. +1.0 |
| FLANG 0 | -0.3 / -2.1 / -5.2 | -1.2 | 10.7 | 10454 / 9544 | differs by up to 5.2 dB, bands -4.2 .. +6.5 |
| FLANG 1 | -0.3 / -2.1 / -5.2 | -1.2 | 10.7 | 10454 / 9544 | differs by up to 5.2 dB, bands -4.2 .. +6.5 |
| CHOR 0 | +0.0 / -2.4 / -4.5 | -2.0 | 11.9 | 10498 / 9972 | differs by up to 4.5 dB, bands -8.5 .. +3.4 |
| CHOR 1 | +0.1 / -2.1 / -3.7 | -1.1 | 8.1 | 10498 / 10852 | differs by up to 3.7 dB, bands -6.0 .. +2.1 |
| VIBE 0 | +0.1 / -2.7 / -3.8 | -0.9 | 6.9 | 10416 / 10715 | differs by up to 3.8 dB, bands -8.3 .. -1.4 |
| VIBE 1 | +0.1 / -2.7 / -3.8 | -0.9 | 6.9 | 10416 / 10715 | differs by up to 3.8 dB, bands -8.3 .. -1.4 |
| ENS 0 | +0.4 / +0.9 / -3.0 | -2.3 | 11.9 | 10498 / 8742 | differs by up to 3.0 dB, bands -8.4 .. +3.5 |
| ENS 1 | +0.4 / +0.9 / -3.0 | -2.3 | 11.9 | 10498 / 8742 | differs by up to 3.0 dB, bands -8.4 .. +3.5 |
| SPIN 0 | +0.2 / -3.0 / -4.5 | -1.3 | 8.9 | 10416 / 9758 | differs by up to 4.5 dB, bands -9.2 .. -0.3 |
| SPIN 1 | +0.1 / -2.3 / -3.0 | -0.4 | 10.0 | 10416 / 9699 | differs by up to 3.0 dB, bands -9.9 .. +0.1 |

### 4.3 Reverb types

Wet level at amount 32 is within 0.4 dB for every type; at 127 the engine FDN is 4 to 17 dB louder, and 20 to 35 dB louder in the 8 to 16 kHz band (the port rolls the top off). The first echo comes 2.6 to 7 times later in the port (89 to 279 ms against 17 to 77 ms; Spring 125 against 18 ms). T60 is shorter in the engine for Spring, Booth and Room (0.18 .. 0.58 s against 0.61 .. 3.18 s) and longer for Stage Chorale, Hall and Cathedral (1.63 .. 3.55 s against 0.99 .. 1.33 s). [reverb-fw-vs-hw](reverb-fw-vs-hw.md) covered the port against hardware; this table is engine against that port.

| kRvbType | Wet level engine - port, noise, amt 32 / 127 (dB) | First echo engine / port (ms) | T60 engine / port (s) | Band delta 31-63 Hz / 8-16 kHz, amt 127 (dB) | Status |
|---|---|---|---|---|---|
| Spring | -0.4 / +9.3 | 17.8 / 125.5 | 0.45 / 3.18 | +10.6 / +20.5 | differs |
| Spring var. | -0.4 / +4.4 | 17.8 / 123.9 | 0.77 / 2.91 | +13.2 / +7.7 | differs |
| Booth | -0.3 / +16.9 | 16.9 / 89.5 | 0.18 / 0.61 | -2.0 / +35.1 | differs |
| Booth Predelay | -0.3 / +16.9 | 51.9 / 187.6 | 0.18 / 0.62 | -2.4 / +33.5 | differs |
| Room | -0.3 / +15.2 | 29.1 / 113.0 | 0.58 / 0.83 | -1.7 / +32.5 | differs |
| Room Predelay | -0.3 / +15.2 | 64.1 / 250.9 | 0.58 / 0.76 | -2.8 / +30.9 | differs |
| Stage | 0 / 0 | - | - | - | not runnable (no firmware path: the port falls back to the engine) |
| Stage Chorale | -0.3 / +14.8 | 43.2 / 112.5 | 1.63 / 0.99 | -1.0 / +30.2 | differs |
| Hall | -0.3 / +16.1 | 59.7 / 198.9 | 3.08 / 1.33 | +2.3 / +29.5 | differs |
| Hall Chorale | -0.3 / +16.3 | 59.8 / 198.9 | 3.18 / 1.32 | +2.2 / +29.3 | differs |
| Cathedral | -0.3 / +16.2 | 77.2 / 278.9 | 3.55 / 1.31 | +4.2 / +29.0 | differs |
| Cathedral Chorale | -0.3 / +16.5 | 77.3 / 278.9 | 3.53 / 1.33 | +4.2 / +29.1 | differs |

### 4.4 Compressor

Engine minus port; the engine level falls with the amount where the port does not, which fits a makeup gain in the firmware routine that the engine lacks (the sign and size are measured; the cause is not isolated).

| kCmpAmt | Slow, noise (dB) | Slow, held notes (dB) | Fast, noise (dB) |
|---|---|---|---|
| 32 | -1.68 | -1.59 | - |
| 64 | -5.35 | -5.25 | -5.35 |
| 127 | -10.87 | -10.74 | - |

### 4.5 EQ (bass, mid, treble at +-12 dB)

Level in 0.1 .. 1.8 dB, shelf corners differ by up to 5 dB in the octave next to each corner (the engine biquads against the firmware tone stack). Not run live (section 5).

| Setting | Level engine - port, noise (dB) | Band delta 31-63 .. 8-16k (dB) |
|---|---|---|
| bass+12 | -0.37 | 0.1 -1.9 -5.0 -3.3 -1.2 -0.5 -0.3 -0.2 -0.2 |
| bass-12 | +0.28 | -0.1 2.8 5.2 3.3 1.2 0.5 0.3 0.2 0.2 |
| mid+12 | -0.47 | -0.0 -0.1 -0.5 -1.3 -0.8 -0.7 -1.1 -0.3 -0.1 |
| mid-12 | +0.11 | 0.0 0.1 0.6 1.4 2.0 0.4 0.5 0.2 0.0 |
| treble+12 | -0.16 | -0.0 -0.0 -0.1 -0.5 -1.7 -4.0 -4.4 -0.7 0.2 |
| treble-12 | +1.83 | 0.0 0.0 0.1 0.4 1.7 3.8 4.7 1.1 -0.3 |

### 4.6 Amp drive and cabinet

| Mode | Level engine - port, noise, drive 0 / 5 / 10 (dB) | Held notes, drive 0 / 5 / 10 (dB) |
|---|---|---|
| Twin I | -4.0 / +12.3 / port silent | +0.6 / +8.6 / port silent |
| Twin II | -3.1 / +4.7 / port silent | -0.5 / +7.2 / port silent |
| JC I | -3.5 / +23.9 / port silent | +5.8 / +15.6 / port silent |
| JC II | -7.0 / +7.2 / port silent | +2.2 / +15.8 / port silent |
| Small I | -3.7 / +4.1 / port silent | +3.0 / +12.0 / port silent |
| Small II | -7.1 / +4.8 / port silent | -1.6 / +9.3 / port silent |
| To Rotary | +0.0 / +0.0 / +0.0 | +0.0 / +0.0 / +0.0 |
| LP24 | +0.0 / +0.0 / +0.0 | +0.0 / +0.0 / +0.0 |
| HP24 | +0.0 / +0.0 / +0.0 | +0.0 / +0.0 / +0.0 |

"port silent" is the finding of section 6.3 (the port output is zero at drive 10 for modes 1 to 6). Modes 7 to 9 (To Rotary, LP24, HP24) take no drive path and are identical.

### 4.7 Delay

| Case | Level engine - port, noise (dB) | First echo engine / port (ms) |
|---|---|---|
| mix64-fb64 | -2.87 | 411.0 / 411.0 |
| mix127-fb64 | +1.10 | 411.0 / 411.0 |

## 5. Not runnable, or not run live

| Item | Why | Next |
|---|---|---|
| Rotary / Leslie | Gate `0x1256AD` is not in the static FX / ORG / PNO images and no recall installs it (D135, D148, D155). Recorded only. | [leslie-gate](leslie-gate.md) follow-ups |
| EQ tone stack | The port stays 2-frame and the live pass is 4 frames; the same granularity fix as Mod 1 was not applied. Engine against port is reported (4.5). | add a 4-frame option as for `FwMod1Rest` |
| Amp drive + cabinet | The live install failed ("pass 0 did not return to the poll loop"); the shaper tables live in the PNO image, not the FX image. | PNO-lane install |
| `FwDelay` | Not wired into `LayerFxChain`; the delay is engine only plus the delay-law port. | wire, then compare |
| Delay feedback FX (types 2 to 11) | No port. Engine rows only in `fx-engine-vs-fw.csv` (`delayfx`). | port first |
| Reverb type 6 (Stage), Spring variation (type 1) live | Type 6 has no firmware path in the port; type 1 live run not done. | port type 6 |

## 6. Findings

### 6.1 Chain gain is 0.1866

The identity stage gives 0.1866 = master gain `DM(0xB651A)` for every stage type, so level deltas in section 3 are against that constant. Confirms D125 (-157.6 dB transfer).

### 6.2 Mod 1 rest types need the 4-frame pass

RM, A-WAH, WAH and PUMP (types 4 to 11) mismatched the oracle by -16 to -26 dB null with the default 2 frames per call. With `mod1_rest_frames = 4` (opt-in, default unchanged) the null is -100 dB or better for all. The routines run 4 frames per live pass; whether the real chip calls them with 2 or 4 frames in the running instrument is open (OQ38).

### 6.3 Amp drive 127 is silent in the port

At `kAmpDrive` 127 the port peak is 0 for kinds 1 to 6 (drive 10 in engine units). `fix_by` out of range returns all ones, giving table index 255 beyond the 129-entry tables. Not checked against the oracle (install failed, section 5), so it may be a port error or an input range the firmware never reaches. Not fixed (OQ38).

### 6.4 Reverb

The port matches the oracle for every runnable type, so the engine FDN differences in 4.3 are engine against firmware: wet level +4 to +17 dB at full amount, 20 to 35 dB more energy above 8 kHz, first echo and T60 differ for every type. Fix would be to use the port (`FirmwareFx::reverb`), already opt-in. No default flip.

### 6.5 Compressor

The port matches the oracle; the engine is 1.7 to 10.9 dB lower (4.4).

### 6.6 Mod 2 and Mod 1 engine responses

The engine differs from the ports in level (up to 5.9 dB at amount 127) and response (octave band spread up to 17.7 dB for PHAS 0, 15 to 17 dB for A-WAH 0 / WAH 1). The ports equal the firmware, so the engine is the one to correct or replace by the port.

## 7. Routing (A / B / master)

The live stage output replaces the FX bus, and every type gives the same chain gain (6.1), so no type-specific A / B / master offset is visible in this harness. The per-slot send levels (kFx1Send and similar) are not exercised by the live chain; they stay in the ARM parameter layer, unchecked here.

## 8. Implications

- Every wired FX port equals the FX chip (match, section 3); the engine path does not, except dry-level cases and the Delay time.
- The ports are therefore the better source for 1:1 parity. Flipping a default to the ports needs the owner's OK (D22, D145); this work changes none.
- The only new runtime option is `LayerFxChain::mod1_rest_frames` (default 2, set 4 to match the live pass).

## 9. Follow-ups

- OQ38: real Mod 1 call count, amp drive 127.
- Amp drive and Leslie need the PNO lane installed on the SHARC oracle.
- 4-frame option for the EQ tone stack and live compare.
- Wire `FwDelay`; port the delay feedback FX; port reverb type 6.
- Reverb Spring variation live.

## 10. Reproduce

```bash
cargo run --release -p nord-tools --bin fx_engine_vs_fw > docs/research/data/fx-engine-vs-fw.csv
cargo build --release -p nord-sharc -p nord-tools
python tools/fx_vs_oracle.py --image FX/core1_sram.bin --out docs/research/data/fx-vs-oracle.csv
```
