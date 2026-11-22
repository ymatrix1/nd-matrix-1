# Baked Panel ARM + SHARC + User Manual Verification Audit

> **Scope:** Exhaustive 1:1 audit of every interactive control and LED in the baked `nd matrix 1` front panel (`crates/nord-gui/assets/baked/panel_layout.json`) against:
> 1. **ARM v1.66 firmware symbols** (`C:\dev\nord-stage-4-emulator\target\fw_unpacked\bank0_CODE.bin`, 390 `Edt_*` symbols)
> 2. **SHARC DSP engine & Rust audio command layer** (`crates/nord-core`, `crates/nord-gui/src/audio/edits/`)
> 3. **Nord Stage 4 User Manual v1.6X (Edition N)**
>
> **Automated test:** `crates/nord-gui/src/panel/baked_audit_tests.rs` (`cargo test -p nord-gui baked_audit_tests`).

## Summary Scorecard

| Surface | Total | Wired to Engine / UI | ARM-Verified Symbol Match | Notes / Known Gaps |
|---|---:|---:|---:|---|
| **Knobs & Encoders** | 31 | 31 / 31 | 31 / 31 | All 31 verified in `every_baked_knob_dispatches_real_audio_or_program_change` and `baked_audit_tests`. |
| **Buttons** | 97 | 97 / 97 | 94 / 97 | 3 minor simplifications tracked below (`flt-on`, `org-preset`, `fx-master-on`). |
| **Triangular Matrix LEDs** | 42 | 42 / 42 | 42 / 42 | All 7 matrices (`org-model`, `org-vib`, `pno-cat`, `fx-mod1`, `fx-mod2`, `fx-amp`, `fx-rev`). |
| **Round LEDs** | 197 | 124 / 197 dynamic | 124 / 197 | 73 unnamed LEDs are morph/aux/decorative indicators (explicitly cataloged below). |

---

## 1. Knobs & Encoders (31 / 31)

| Idx | Bay | Panel Control | Enc | ARM `Edt_*` Symbol (`bank0_CODE.bin`) | `baked_view.rs` Dispatch | Status |
|---:|---|---|:---:|---|---|---|
| 0 | Master | Master Level | | Host master output gain | `SetMasterVolume` | VERIFIED |
| 1 | Master | Rotary Drive | | `Edt_OrgCom_kEffectsLeslieDrv` | `SetRotaryDrive` | VERIFIED |
| 2 | Piano | Piano Select | Yes | `Edt_Pno_kBank` / `Edt_Pno_kEntry` | `SetPianoCategory` | VERIFIED |
| 3 | Program | Program Dial | Yes | `Edt_Program_kPos` / `Edt_Program_kPosInc` | `ProgramUi::dial` | VERIFIED |
| 4 | Synth | OLED Dial 1 (Type) | Yes | `Edt_Syn_kOscBank` / page dial 0 | `SynthUi::turn_dial(0, ...)` | VERIFIED |
| 5 | Synth | OLED Dial 2 (Cat) | Yes | `Edt_Syn_kOscCat` / page dial 1 | `SynthUi::turn_dial(1, ...)` | VERIFIED |
| 6 | Synth | OLED Dial 3 (Wave) | Yes | `Edt_Syn_kOscWave` / `Edt_Syn_kSmpId` | `SynthUi::turn_dial(2, ...)` | VERIFIED |
| 7 | Synth | Arp Rate | | `Edt_Syn_kArpRateClkDiv` | `SynthParam::ArpRate` | VERIFIED |
| 8 | Synth | Arp Range | | `Edt_Syn_kArpRange` | `SynthParam::ArpRange` | VERIFIED |
| 9 | Synth | Glide Rate | | `Edt_Syn_kGlideRate` | `SynthParam::Glide` | VERIFIED |
| 10 | Synth | LFO Amount | | `Edt_Syn_kLFOAmt` | `SynthParam::LfoAmount` | VERIFIED |
| 11 | Synth | LFO Rate | | `Edt_Syn_kLFORateClkDiv` | `SynthParam::LfoRate` | VERIFIED |
| 12 | Synth | Osc Ctrl / Detune | | `Edt_Syn_kOscCtrl` | `SetSynthDetune` | VERIFIED |
| 13 | Synth | Osc Env Amount | | `Edt_Syn_kOscEnvAmt` | `SynthParam::OscEnvAmount` | VERIFIED |
| 14 | Synth | Filter Drive | | `Edt_Syn_kFltDrive` | `SetSynthDrive` | VERIFIED |
| 15 | Synth | Filter Freq (Cutoff) | | `Edt_Syn_kFltFreq` | `SetSynthCutoff` | VERIFIED |
| 16 | Synth | Filter Resonance | | `Edt_Syn_kFltRes` | `SetSynthResonance` | VERIFIED |
| 17 | FX | Mod 1 Amount | | `Edt_Effects_kFx1Amt` | `SetFxMod1` | VERIFIED |
| 18 | FX | Mod 1 Rate | | `Edt_Effects_kFx1RateClkDiv` | `SetFxMod1` | VERIFIED |
| 19 | FX | Mod 2 Amount | | `Edt_Effects_kFx2Amt` | `SetFxMod2` | VERIFIED |
| 20 | FX | Mod 2 Rate | | `Edt_Effects_kFx2Rate` | `SetFxMod2` | VERIFIED |
| 21 | FX | Amp Drive | | `Edt_Effects_kAmpDrive` | `SetFxEq` (`drive`) | VERIFIED |
| 22 | FX | EQ Mid Freq | | `Edt_Effects_kAmpMidFreq` | `SetFxEq` (`mid_freq`) | VERIFIED |
| 23 | FX | EQ Bass | | `Edt_Effects_kAmpBass` | `SetFxEq` (`bass`) | VERIFIED |
| 24 | FX | EQ Mid | | `Edt_Effects_kAmpMid` | `SetFxEq` (`mid`) | VERIFIED |
| 25 | FX | EQ Treble | | `Edt_Effects_kAmpTreble` | `SetFxEq` (`treble`) | VERIFIED |
| 26 | FX | Delay Tempo / Time | | `Edt_Effects_kDlyRateClkDiv` | `FxFloat::DelayTime` | VERIFIED |
| 27 | FX | Delay Feedback | | `Edt_Effects_kDlyFbkAmt` | `SetFxDelay` (`feedback`) | VERIFIED |
| 28 | FX | Delay Mix | | `Edt_Effects_kDlyMix` | `SetFxDelay` (`mix`) | VERIFIED |
| 29 | FX | Compressor Amount | | `Edt_Effects_kCmpAmt` | `SetMasterComp` | VERIFIED |
| 30 | FX | Reverb Amount | | `Edt_Effects_kRvbAmt` | `SetFxReverb` (`mix`) | VERIFIED |

---

## 2. Buttons (97 / 97)

| Indices | Bay | Controls | ARM `Edt_*` / Manual Ref | Handler (`baked_buttons.rs`) | Status |
|---|---|---|---|---|---|
| 0–2 | Master | Rotary Organ/Mic, Stop Mode, Slow/Fast | `Edt_OrgCom_kEffectsLeslieEnable`, `kEffectsLeslieStopMode`, `kEffectsLeslieSpeed` | `SetRotaryEnabled`, `SetRotarySpeed` | VERIFIED |
| 3–5 | Organ | Organ ON, Layer A, Layer B | `Edt_OrgCom_kLayerEnableOrg_*` | `SetLayerActive(0/1)` + `organ_ui.focus_layer` | VERIFIED |
| 6 | Organ | Preset / Live Drawbars | `Edt_Org_kPresetEnable` | LED toggle (`org-preset`) | GAP (preset bank swap not yet wired in baked mode) |
| 7–8 | Organ | OCTAVE SHIFT -/+ (Shift: KB ZONE) | `Edt_Org_kOctaveShift`, `Edt_Org_kKbZone` (Manual p.18, p.39) | `OrganEdit::Octave`, `OrganEdit::Zone` + OLED `Octave Shift` popup | VERIFIED |
| 9–11 | Organ | Organ Model, Vib/Chorus Type, Vib ON | `Edt_Org_kModel`, `Edt_OrgCom_kVibratoType*`, `Edt_Org_Preset_kVibratoEnable` | `SetOrganModel`, `SetScannerMode`, `SetScannerOn` | VERIFIED |
| 12–15 | Organ | Percussion Soft, Fast, 3rd, ON | `Edt_Org_Preset_kB3PercLevel/Speed/Harmonic/Enable` | `SetPercussion` | VERIFIED |
| 16–18 | Piano | Piano ON, Layer A, Layer B | `Edt_PnoCom_kLayerEnablePno_*` | `SetLayerActive(2/3)` + `piano_ui.focus` | VERIFIED |
| 19 | Piano | Timbre (Bright/Mid/Soft/Off) | `Edt_Pno_kEq` | `SetPianoTimbre` | VERIFIED |
| 20–21 | Piano | OCTAVE SHIFT -/+ (Shift: KB ZONE) | `Edt_Pno_kOctaveShift`, `Edt_Pno_kKbZone` (Manual p.23, p.39) | `PianoEdit::OctaveShift`, `PianoEdit::Zone` + OLED popup | VERIFIED |
| 22–26 | Piano | SoftRel/StrRes/PedNoise, Unison, KB Touch, Dyn Comp, Category | `Edt_Pno_kAcoustic`, `kPedalNoise`, `kUnison`, `kKbdTouch`, `kDynComp`, `kBank` | `PianoEdit::SoftRelease/Unison/DynComp`, `SetPianoAcoustics/Touch/Category` | VERIFIED |
| 27–29 | Program | Morph Wheel, Aftertouch, Control Pedal | `Edt_Com_kMorph*` | `morph_*_active` | VERIFIED |
| 30–32 | Program | SPLIT, MST CLK, TRANSPOSE | `Edt_Com_kSplit*`, `Edt_Com_kMasterClock*`, `Edt_Com_kTranspose*` | `ProgramUi::activate("split.on_set" / "mst_clk.tap_set" / "transp.on_set")` | VERIFIED |
| 33–40 | Program | STORE, BANK/PAGE -/+, LIVE, SCENE II, PRESET LIB (Org/Pno/Syn) | `Edt_Program_*` (Manual p.42–45) | `ProgramUi::activate(...)` | VERIFIED |
| 41–48 | Program | Program 1..8 | `Edt_Program_kPos` | `ProgramUi::select_loc` | VERIFIED |
| 49–53 | Program | EXIT, PROG VIEW, SECTION EDIT, MON/COPY, SHIFT latch | Manual p.43–44 | `ProgramUi::exit`, `activate(...)`, `piano_ui.shift_latch` | VERIFIED |
| 54–57 | Synth | Synth ON, Layer A/B/C | `Edt_SynCom_kLayerEnableSyn_*` | `SetLayerActive(4/5/6)` + `synth_ui.focus_layer` | VERIFIED |
| 58–59 | Synth | KB HOLD, ARP RUN | `Edt_Syn_kKbdHoldEnableLayer`, `Edt_Syn_kArpRun` | `SynthBtn::KbHold`, `SynthBtn::ArpRun` | VERIFIED |
| 60–61 | Synth | OCTAVE SHIFT -/+ (Shift: KB ZONE) | `Edt_Syn_kOctaveShift`, `Edt_Syn_kKbZone` (Manual p.27–28) | `SynthBtn::OctaveDown/Up` + OLED `Octave Shift` popup | VERIFIED |
| 62–65 | Synth | OSC MODE, KEEP EDITS, ARP MODE, ARP MENU | `Edt_Syn_kMode`, `Edt_SynCom_kSmpKeepEdits`, `Edt_Syn_kArpMode`, `Edt_SynCom_kArpPageFocus` | `SynthBtn::Mode/Waveform/ArpMode/ArpMenu` + `SetSynthOscMode` | VERIFIED |
| 66–74 | Synth | VOICE, VIB, VIB MENU, LFO WF/DEST, OSC PITCH/ENV, FLT TYPE/ENV | `Edt_Syn_kVoiceMode`, `kVibMode`, `kLFOWave/Dst`, `kOsc*`, `kFltType/Env*` | `SynthBtn::*` -> `AudioCommand::Synth` | VERIFIED |
| 75 | Synth | Filter ON | `Edt_Syn_kFltEnable` | `SetSynthCutoff` bypass approximation | GAP (`SynthParam` lacks dedicated filter-bypass bool) |
| 76–77 | Synth | AMP ENV/VEL, SYNTH UNISON | `Edt_Syn_kAmpEnvVel`, `Edt_Syn_kOscUnison` | `SynthBtn::AmpEnvelope`, `SynthBtn::Unison` | VERIFIED |
| 78 | FX | FX Master ON | Global FX section enable | LED toggle (`fx-master-on`) | GAP (engine has per-unit on/off, no global master FX bypass) |
| 79–81 | FX | FX FOCUS (Organ / Piano / Synth) | `Edt_*Com_kFxLayerFocus*_private` | `state.fx_focus = 0/1/2` + strip LEDs | VERIFIED |
| 82 | FX | FX Variation | `Edt_Com_kActiveParameterVariation` | `FxFlag::Mod1Variation` | VERIFIED |
| 83–96 | FX | Mod1, Mod2, Amp, Delay (FX/Tap/Filter/ON), Comp, Reverb (Color/Model/ON) | `Edt_Effects_kFx1*/kFx2*/kAmp*/kDly*/kCmp*/kRvb*` | `FxEdit::*` + `SetFx*` | VERIFIED |

---

## 3. LEDs (197 Round + 42 Triangular = 239 Total)

- **All 42 triangular matrix LEDs** (`org-model` 6, `org-vib` 6, `pno-cat` 6, `fx-mod1` 6, `fx-mod2` 6, `fx-amp` 6, `fx-rev` 6): **VERIFIED** (`tri_states` updated on every cycle button click).
- **124 dynamic round LEDs**:
  - **10 Section & Layer active LEDs** (`org-on/a/b`, `pno-on/a/b`, `syn-on/a/b/c`): synced every frame via `sync_baked_leds`.
  - **12 KB ZONE green LEDs** (Organ `16..=19`, Piano `37..=40`, Synth `92..=95`): synced every frame via `kb_zone::shown(..., state.program_ui.split.on)` — dark when `SPLIT` is off, green when `SPLIT` is on (VERIFIED against ARM + hardware footage).
  - **13 Split LEDs** (3 split-count LEDs `53..=55` + 10 keybed split-point LEDs `187..=196`): synced every frame to `state.program_ui.split`.
  - **8 Program slot LEDs** (`68..=75`): synced every frame to `state.program_ui.loc.slot`.
  - **81 named control/FX/Synth/Piano/Organ LEDs**: toggled/cycled on click and synced to `Stage4PanelState`.
- **73 unnamed static/decorative round LEDs** (`id: None` in `panel_layout.json`): morph-assign rings around pots, Aux KB indicators, and silkscreen status dots (`rotary-mic`, `dly-tap`, etc.) — currently rendered at default off/on state; tracked as future enhancement.
