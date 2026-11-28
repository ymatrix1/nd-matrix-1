# Front-Panel Visual Style & UI Specification

## 1. Scope

Specifies the visual layout, colour palette, typography, widgets, LED indicators, OLED displays, and interactive behaviours for the ND Matrix 1 GUI (Standalone & CLAP/VST3 DAW Plugin).

**Identity (D251, 2026-11-23).** The look is an original design, the **Slate Matrix** identity, and deliberately does not imitate the vendor's red chassis or its control styling (trade dress, `ns4-80xe`). Control *positions*, counts, indices and parameter IDs are unchanged (D16: saved DAW projects keep loading); only surfaces, shapes, colours, texture and typography differ. Which control exists where and what it does follows the instrument's function as documented in its manual; no vendor art, photo, trace or colour sample is used. Photos in `.reference/` are measurement-only (control geometry, scale sweeps) and never ship (`tools/check_no_vendor_photos.py`).

### 1.1 Slate Matrix: what differs from the original instrument

| Element | Original instrument | ND Matrix 1 |
|---|---|---|
| Chassis | crimson lacquer, red-stained wood cheeks | graphite-navy powder coat with a fine **dot-matrix** texture; graphite side cheeks with a teal edge line |
| Module bays | speckled blue-grey slate | navy slate with the dot-matrix texture |
| Section headers | light silver bar, rounded stepped tab | **dark plate with 45 degree chamfers** and a 3 px **section-accent underline** (Organ amber, Piano mint, Program cyan, Synth violet, Effects sky) |
| Light sub-panels | silver-blue | **bone** (`#D3CCB9`) with dark ink |
| Pot knobs | round, fluted, white wrap-over line | **hexagonal** graphite body, brushed-aluminium round cap, **cyan pointer** |
| Encoders | round, silver ring | round, **teal ring** |
| Buttons | black stadium pill on a rectangular collar | **square-cornered tile cap with a centre groove**; graphite, bone and cyan caps |
| Highlighted buttons / LEDs | red | **cyan** (mint for ladders, amber kept for Morph) |
| Faders | grey pyramid cap | graphite cap with a cyan index line |
| Drawbars | white and black caps | **sand** and **navy** caps with a ribbed grip band |
| Keys | white and black, red felt lip | **ivory** and **blue-black** keys, near-black felt, pressed keys tint pale teal |
| Pitch stick / mod wheel | wooden stick, pebbled stone wheel | aluminium stick with a cyan line, ribbed metal wheel |
| Typography | proportional DIN-style sans | **monospace** silkscreen (`Cascadia Mono` / `Consolas`), upper-case labels |
| Wordmark | lower-case `nd matrix 1` | 3x3 dot-grid glyph (one lit dot) plus `ND MATRIX 1`, letter-spaced |

Guards: `crates/nord-gui/src/identity_tests.rs` (no red-dominant chassis token, no red-dominant pixel in the baked plate or sprite atlas) and `tools/tests/test_bake_panel_from_html.py` (no legacy red tokens in the prototype CSS, hexagon knob, no stadium pills). Silkscreen text resolves against the fonts installed on the baking machine; the committed PNGs are the source of truth.

---

## 2. Color Palette & Surface Tokens

All values are `ASSUMED` design choices (no hardware measurement); the checked property is text contrast. The same values live in `prototypes/panel-mock/styles.css` (`:root`, source of the baked plate) and `crates/nord-gui/src/style.rs` (vector fallback panel, plugin chrome).

| Token (CSS / Rust) | Hex | Usage |
|---|---|---|
| `--chassis-base` / `CHASSIS_BASE` | `#151C29` | Outer chassis, master strip, program deck |
| `--chassis-dark` / `CHASSIS_DARK` | `#0B1018` | Recessed borders, seams, plugin bar fill |
| `--module-slate-bg` / `MODULE_SLATE` | `#243249` | Background of `ORGAN`, `PIANO`, `SYNTH`, `LAYER EFFECTS` |
| `--module-slate-dark` / `MODULE_SLATE_DARK` | `#1A2538` | Recessed frames |
| `HEADER_DARK`, `HEADER_INK`, `HEADER_EDGE` | `#0D121B`, `#E8ECF0`, `#2B3A52` | Section header plate, ink (15:1), hairline |
| `--bone` / `MODULE_HEADER_BAR` | `#D3CCB9` | Light sub-panels, Rotary Speaker tab |
| `--group-frame-border` / `GROUP_BORDER` | `#7F97BA` | 1 px outline boxes (`ORGAN MODEL`, `VIB/CHORUS`, `MOD 1`, ...) |
| `BUTTON_GREY` | `#CFC8B5` | Bone layer / toggle buttons |
| `BUTTON_BLACK` | `#1B2231` | Graphite selector buttons |
| `--accent` / `LED_ACCENT`, `BUTTON_ACCENT` | `#35E0E8`, `#35D9E1` | Active LEDs, drawbar ladders, triangles, STORE and ARP RUN caps, knob pointer |
| `LED_GREEN` | `#3DE8A8` | 11-segment fader ladders, `ON/OFF` halos, `KB ZONE` indicators |
| `LED_AMBER` | `#FFB020` | Morph assignment, secondary status |
| `OLED_BG` / `OLED_FG` | `#06090E` / `#BFF6F2` | OLED background and pale-cyan pixels |
| `DRAWBAR_WHITE` / `DRAWBAR_BLACK` | `#E6DCC0` / `#1F2A3F` | Sand and navy drawbar caps |
| `KEY_WHITE` / `KEY_WHITE_PRESSED` | `#F7F1E0` / `#D3EFE9` | Ivory key and pressed tint |
| Section accents | Organ `#F0A63C`, Piano `#2FD0B8`, Program and Master `#35E0E8`, Synth `#9A87FF`, Effects `#4CB0FF` | Header underline per bay |

---

## 3. Left-to-Right Panel Architecture (6 Horizontal Bays)

The top panel is laid out horizontally above the 88-key triple-sensor keybed (with split-point LEDs above the keybed lip):

```text
┌──────────┬────────────────────┬───────────────────┬────────────────────┬─────────────────────────┬───────────────────────┐
│  1. LEFT │  2. ORGAN SECTION  │ 3. PIANO SECTION  │ 4. PROGRAM / MORPH │    5. SYNTH SECTION     │   6. LAYER EFFECTS    │
│  MASTER  │   (Slate #243249)  │  (Slate #243249)  │   (Navy #151C29)   │     (Slate #243249)     │    (Slate #243249)    │
│ & ROTARY │                    │                   │                    │                         │                       │
│ (#151C29)│ • Faders A & B     │ • Faders A & B    │ • MORPH ASSIGN     │ • Faders A, B & C       │ • FX FOCUS (Org/Pno/  │
│          │   + 11-LED ladders │   + 11-LED ladders│   (Wheel/AT/Pedal) │   + 11-LED ladders      │   Syn A/B/C, Group)   │
│ • Pitch  │ • ORGAN MODEL      │ • ACOUSTICS       │ • SPLIT (3 zones)  │ • SYNTH OLED DISPLAY    │ • MOD 1 (Rate/Amount) │
│   Stick  │   (B3/Vox/Farf/    │   (String Res,    │ • MST CLK / TRANSP │   (Waveform, Detune,    │ • MOD 2 (Rate/Amount) │
│ • Mod    │    Pipe1/Pipe2)    │    Pedal Noise,   │ • MAIN OLED SCREEN │    Mode soft-tabs)      │ • AMP SIM / 3-Band EQ │
│   Wheel  │ • VIB/CHORUS       │    Soft Rel)      │   (Program #/Name, │ • MODE (Sample/Analog/  │   (Drive, Bass, Mid   │
│ • Master │   (V1-V3, C1-C3)   │ • UNISON (1/2/3)  │    Layer Summary)  │    Wavetable/FM)        │    Gain+Freq, Treble) │
│   Level  │ • B3 PERCUSSION    │ • KB TOUCH        │ • LIVE MODE        │ • ARPEGGIATOR / GATE    │ • DELAY (Tempo,       │
│ • Rotary │   (Soft/Fast/3rd)  │ • DYN COMP        │ • LAYER SCENE II   │ • LFO & OSCILLATORS     │   Feedback, PingPong, │
│   Drive  │ • 9 PHYSICAL       │ • TIMBRE          │ • PROGRAM 1..8     │ • FILTER (LP12/24/Lad/  │   Analog, Filter, Mix)│
│ • Slow / │   DRAWBARS +       │ • PIANO SELECT    │   Selector Buttons │   BP/HP, Freq, Res)     │ • COMP (Amount, Fast) │
│   Fast   │   8-LED LADDERS│   + MODEL Encoder │                    │ • AMP ENV & UNISON      │ • REVERB (6 Rooms,    │
│          │ • KB ZONE (4 LEDs) │ • KB ZONE (4 LEDs)│                    │ • KB ZONE (4 LEDs)      │   Bright/Dark, Chorale│
└──────────┴────────────────────┴───────────────────┴────────────────────┴─────────────────────────┴───────────────────────┘
```

**Keybed mouse velocity (D84).** A mouse press on the on-screen keybed sends velocity from where the pointer hits the key: `v = 20 + t * 107` (rounded), `t = y / key_length` clamped to 0..1, `y` measured from the top (back) of the key. White key length = keybed height; black key length = `0.62 * height`. Back = 20, front tip = 127 (ASSUMED ends and linear law; `keybed_velocity.rs`). The computer keyboard (default 88, C / V steps) and MIDI In keep their own velocity.

---

## 4. Custom Hardware Widget Specifications

Every custom UI widget in `crates/nord-gui` / `crates/nord-plugin` must render the exact hardware cues shown in `NS4-Panel-organ.max-2000x2000.format-png.png` and `Screenshot20at2015.jpg`:

### 4.1 Layer Volume Fader + 11-Segment Green LED Ladder (`LayerFaderWidget`)
- **Physical Fader:** Vertical slot track with a sculpted matte-grey rectangular fader cap (`#8C9196` with top/bottom bevel highlights).
- **11-Segment Green LED Bar (`#5CE62E`):** Positioned immediately to the right of each fader track. Shows the effective layer level (including active **Morphing** offsets in real time!) so the player sees both the physical fader position and the morphed level simultaneously.
- **Layer Activation Button (`A`, `B`, `C`):** Above each grey rectangular button sits a pill label (`ON/OFF ▼`) with a circular green ring LED (`#5CE62E` when active in the current Scene, dim when off, blinking when edited in FX Focus).
- **Zone Indicator (`◄ KB ZONE ►`):** 4 circular green LEDs at the bottom of each section's mixer strip showing which of the 4 split zones the selected layer spans.

### 4.2 Physical Organ Drawbar + 8-Segment LED Ladder (`DrawbarWidget`)
- **9 Drawbar Tracks (`16'`, `5 1/3'`, `8'`, `4'`, `2 2/3'`, `2'`, `1 3/5'`, `1 1/3'`, `1'`):**
  - Pulled **down** (`0` at top, `8` fully pulled out at bottom), matching real Hammond B3 / Nord Stage 4 mechanics.
  - Top labels show Pipe/Farfisa/Vox equivalents (`BASS16 / 16'`, `STR16 / 8'`, `FLUTE8 / 4'`, `OBOE8 / 2'`, `TRMP8 / II`, `STR8 / III`, `FLUTE4 / IV`, `STR4`, `2 2/3 ~`).
  - Bottom labels show B3 harmonic footages (`16'`, `5 1/3'`, `8'`, `4'`, `2 2/3'`, `2'`, `1 3/5'`, `1 1/3'`, `1'`).
- **Adjacent 8-Segment LED Ladder (`1..8`):**
  - Next to every drawbar slot is a white-framed vertical strip numbered `1` through `8` with 8 rectangular accent LEDs (`#35E0E8`).
  - Lights from the top down (`1..N`) to display the active drawbar registration (and any live Morph transition).
  - Unlit while the Organ section is off and in Drawbar Live (EMPIRICAL: the hardware frame of the piano program `A:13 Felt Upright` shows no lit ladder, [D78](../DECISIONS.md)).
- **Drawbar Cap Colors:**
  - Indices `0, 1` (`16'`, `5 1/3'`): Black (`#1A1C1E`)
  - Indices `2, 3` (`8'`, `4'`): White (`#ECEBE6`)
  - Index `4` (`2 2/3'`): Black (`#1A1C1E`)
  - Index `5` (`2'`): White (`#ECEBE6`)
  - Indices `6, 7` (`1 3/5'`, `1 1/3'`): Black (`#1A1C1E`)
  - Index `8` (`1'`): White (`#ECEBE6`)

### 4.2a Program load -> panel state, checked against the demo video (`ns4-6c08`)

Ground truth: frames of `NEW NORD STAGE 4 - PROGRAMAS PRESETS DEMO (PART A)` (Stage 4 Compact, bank A in order; frames and the backup stay in git-ignored `.reference/` and `target/video_frames/`). Reproduce ours with `python tools/gui_shots.py --backup <ns4b> --program A:14 --sections organ` (`--program` takes a panel location; `nd-matrix-1 --backup X --program A:14 --screenshot out.png` does the same). A location is `bank:page+button`, the file's `location_index` 3 is `A:14` (checked for A:14, A:24, A:34, A:77 against the names on the OLED).

| Rule | Evidence | Tag |
|---|---|---|
| The Organ ladders, percussion LEDs, model, vibrato ON and KB ZONE LEDs show the **focused layer**; with both layers on a load focuses B, with only A on it focuses A | A:14, A:24, A:34 show the stored layer B values; A:77 (A only) shows A | VERIFIED (4 programs); the firmware stores the focus (`engine-routing.md`: `B3 Chords n Solo` focus 1), its bits are not decoded: Open |
| Drawbar ladder `n` = `n` LEDs lit from the top (`1..n`), `0` = none, `8` = all | A:14 layer B `8 8 8 6 3 3 1 5 0` counted on the frame at 3:56 | VERIFIED |
| The 9 sliders are physical: a program load never moves them (all caps sit at value 8 in every frame); a slider edit in Preset mode writes the focused layer's preset | every frame of the video | VERIFIED |
| Percussion ON / SOFT / FAST / THIRD are stored **per layer** (the engine plays both sets) | A:14: layer B `on, 3rd` lit, layer A stores `off, soft, fast, 3rd` | VERIFIED |
| The focused layer's ON/OFF LED blinks while several layers of the section are on, the other layers stay lit | 10 frames of A:14 at 2 fps: B alternates, A steady | VERIFIED (Organ); same rule in Piano and Synth: ASSUMED |
| KB ZONE LEDs = `CASE_LEDS[CASE_TABLE[active split points][code]]` of the focused layer, **all dark when the split is off** | A:14 B code 6 -> zones 3-4, A:34 B code 0 -> zones 1-2 (middle split point only); no split (A:13, A:38, A:77): dark | VERIFIED |
| STRING RES / PED NOISE / SOFT REL LEDs only on Grand and Upright | A:13 lit; A:22 (Electric) dark although the flags are stored on | VERIFIED for Electric, ASSUMED for the rest |
| Rotary: speed, stop, drive, ORGAN routing follow the program | decoded; the video user switches speed live, not compared | ASSUMED |

Open (not matched yet): Piano TIMBRE of `Felt Upright` shows SOFT while the stored `kEq` is 0 (`Off`); UNISON / DYN COMP LEDs of A:13 show lit points that the stored 0 does not explain; SUSTPED / PSTICK LEDs per layer (A:38, A:13) are not decoded; the stored organ focus bits; Piano and Synth focus rules (A:38 Synth focuses A with A and B on, so "highest layer" is Organ only).

### 4.3 Dual-Column Triangle Matrix Selector (`TriangleSelectorWidget`)
- Used in `ORGAN MODEL` (`FARF / VOX / B3` vs `PIPE1 / PIPE2 / B3 BASS`), `VIB/CHORUS` (`C2 V3 C3 / V2 C1 V1`), and `PIANO SELECT` (`ELECTRIC / UPRIGHT / GRAND` vs `CLAV / DIGITAL / MISC`).
- Recessed dark bezel containing left/right pointing accent LED triangles (`◄` `►`) above a black pill selector button.

### 4.4 Rotary Knobs & Morph Range Arcs (`NordKnobWidget`)
- Dark charcoal ribbed cylindrical cap with a crisp white radial pointer line (or endless stepped encoder with `LIST` badge for `MODEL` / `OSC`).
- **Morph Assignment Visualization:** Holding `WHEEL`, `A.T.`, or `CTRLPED` in the `MORPH ASSIGN` bar (or hovering a morphed knob) draws a glowing arc around the knob perimeter from `base` to `base + delta`, allowing instant visual creation and inspection of Morph ranges just like LED-ring encoders on hardware.

### 4.5 Dual Monochrome OLED Displays (`ProgramOledWidget` & `SynthOledWidget`)
1. **Central Program OLED (`128×64` aspect):**
   - Top row: Bank & Location (`A:11`) + Program Title (`Nord Stage 4`) + Category icon.
   - Edited marker: a boxed `E` follows the location while the program differs from what was loaded or stored (VERIFIED, course footage and manual p.13). The panel counts the sound edits it sends (`AudioHandle::edit_count`, `AudioCommand::is_program_edit`) and compares the count with its value at the last load, program change or STORE (`bay_program/edited.rs`). Setters sent with the same value still count (ASSUMED). In Live Mode the title is `Live N` and no `E` shows (ASSUMED: Live programs auto-store).
   - Section rows (Layers view): one row per section with a layer on (`Organ B3`, `Piano ...`, `Synth ...`). The Piano row ends with the size letter of the loaded library file (`Pearl Upright  L`; `S`, `M`, `L`, `XL` from the `.npno` header name, `VERIFIED`, see [piano-types](../research/piano-types.md)) and the B3 Organ row with the drawbar positions as nine digits (`Organ B3 888000000`, `ASSUMED` text form of the mini drawbars; Vox, Farfisa and Pipe show none). The preset id (`1:05`) the hardware prints before a preset name is not shown: a program stores no preset link and the emulator does not load `.ns4o` / `.ns4n` / `.ns4y` presets (bean `ns4-bjvn`).
   - Bottom 3 rows: Active Organ model/drawbar preview (`888000000`), active Piano model (`White Grand XL`), and active Synth patch (`Vista Pad WH`) with `A/B/C` layer badges.
2. **Synth Section Graphic OLED (`128×64` aspect):**
   - Header: `OSC WAVEFORM`
   - Center graphic: Real-time anti-aliased waveform / wavetable / FM operator visualization + primary parameter readout (`Super Saw`, `DETUNE: 3.4`).
   - Bottom bar: 4 inverted/outlined soft-label tabs (`ANALOG`, `SUPER`, `1 (6)`, `WAVE`) controlled by the 3 encoders below the screen.

#### Parameter hint line (`ns4-wxly`)

While a knob moves, both OLEDs show `<label> <value>` (`LFO Rate 0.08 Hz`, `LFO Amount 7.1`, `Filter Frequency 1.2 kHz`) for 1.5 s after the last movement (hold time `ASSUMED`, no timing measured).

| Display | Where the hint goes | Source |
|---|---|---|
| Synth OLED | replaces the page title in the header row (right-hand layer letter stays) | LFO course video 0:27-0:45 (EMPIRICAL) |
| Program OLED | replaces the rows below the separator on the home screen; menus, STORE, LIST and split screens keep their text | manual p.42 (VERIFIED) |

Data: `panel/param_hint/table.rs` maps an inventory control id to label and value formatter (`fmt.rs`); every `widgets::knob` drag records `(id, position)` in the egui context (`param_hint::touch`), owners with a non-linear value (Master Clock divisions, stored layer value, per-type mod rate) refine it with `touch_value`. The LFO DEST button taps raise `LFO Destination <Filter|Osc Pitch|OscCtrl|Off>`.

| Item | Status |
|---|---|
| Labels `Oscillator Ctrl`, `Oscillator Env Amount`, `Filter Frequency`, `Filter Resonance`, `Filter Env Amount`, `LFO Destination`, `LFO Rate`, `LFO Amount`, `Glide Rate`, `Arpeggiator Rate`, `Arpeggiator Range`, `Filter`, `Osc Pitch`, `OscCtrl` | VERIFIED in the firmware pool, `tools/tests/test_ns4_oled_strings.py::test_param_hint_labels_are_in_the_firmware` |
| Effects and Master labels (`Amp Bass`, `Delay Feedback`, `Master Level`, ...), `Off` | ASSUMED: not firmware strings; built from the panel silkscreen |
| Hz precision (2 decimals below 1 Hz, 1 below 10 Hz, 0 above) | EMPIRICAL: LFO clip overlay readings, [dsoundman-synth-lfo](../research/dsoundman-synth-lfo.md) section 2 |
| `dB` with one decimal, `bpm` integer | VERIFIED unit strings (`%.1f dB`, `%d bpm`, `0x2CA110..`) |
| Panel-unit scale `0..10` (osc ctrl, resonance, LFO amount), `-10..10` (env amounts) | EMPIRICAL for LFO amount (`7.1`, one reading); ASSUMED for the others |
| Knob laws (log or linear sweeps and their ranges) | ASSUMED, as in `ui_state/synth/pots.rs` |
| Open | The hardware LFO RATE table (`LFO_RATE_HZ_TABLE`, 128 steps) is not used for the hint: it shows the emulator's own rate (log 0.05..40 Hz); moving the knob law to the table is a separate change |

#### 4.5.1 Firmware messages and toasts (`bay_program/fw_messages.rs`, bean `ns4-0u5u`)

The Program OLED shows short messages ("toasts") for events. They use the firmware's own strings; every offset is a file offset in `bank0_CODE.bin` (OS v1.66, local only), checked by `tools/tests/test_ns4_oled_strings.py::test_message_strings_are_in_the_firmware`.

| Message | Offset | Shown when | Status |
|---|---|---|---|
| `Piano not found` | `0x2CA180` | a program load finds an enabled piano layer whose model the opened `.ns4b` lacks, or a piano is requested with no archive open | string `VERIFIED`, trigger `ASSUMED` |
| `Sample not found` | `0x2CA198` | same for an enabled synth layer in Sample mode | string `VERIFIED`, trigger `ASSUMED` |
| `Please wait...` | `0x2C75D0` | a piano / sample decode is running (`pending_jobs > 0`); removed when it ends, never hides another message | string `VERIFIED`, trigger `ASSUMED` |
| `Pasted` / `Swapped` | `0x283758` / `0x283760` | Paste / Swap soft buttons with a filled clipboard | `VERIFIED` |
| `Nothing to Paste` / `Nothing to Swap with` | `0x28377D` / `0x2837E5` | Paste / Swap soft button with an empty clipboard | `VERIFIED` |
| `Are you sure?` | `0x2C0E38` | first press of System > Reset all settings (`0x2C0E08`, the string right before it); a second press within 2 s runs it | string `VERIFIED`, pairing with Reset `ASSUMED` (neighbouring strings, caller not traced) |
| `Unable to Paste` + `Memory Protected` | `0x283790` (one string with `\n%c Memory Protected`) | pasting a whole program while Memory Protect is on | string `VERIFIED`, use `ASSUMED` |
| `Memory Protected` | `0x2837A0` (`%c Memory Protected`) | STORE / Organize blocked by Memory Protect | `VERIFIED` |

`%c` in `%c Memory Protected` is the character code `0x89` (`mov r3, #0x89` before the print call at `0xC003D32C` and `0xC003D344`, `VERIFIED` by `python tools/arm_xref.py dis target/fw_unpacked/bank0_CODE.bin 0xC003D2E0 0xC003D360`), a glyph of the panel's OLED font. Its shape is not decoded (a padlock is `ASSUMED` from context); the emulator font has no such glyph, so only `Memory Protected` is drawn.

Mechanism: `ProgramUi::set_message` / `show(FwMessage)` store `(text, end_ms)` and `begin_frame(now_ms, ..)` drops the message at `end_ms`, so the lifetime follows the injected frame clock and tests pass a time instead of sleeping. The lifetime `TOAST_MS = 2000` is `ASSUMED`. A message with a line break is drawn as one OLED row per line. `ProgramUi::confirm(Confirm)` is the two-press prompt: the first press shows `Are you sure?` and arms the action for `TOAST_MS`, the same action again inside the window returns `true`. Library loads queue `FwMessage`s in `BackupLibraryState` and the panel poll turns them into toasts.

### 4.6 Known discrepancies (open, Phase 6b `ns4-xc5n`)

| Item | This spec says | Manual / evidence | Status |
|---|---|---|---|
| Morph target display (§4.4) | Amber glowing arc around the knob | Manual p.11: green Morph LEDs at the bottom-left of the knob show morph assignment | Spec wrong; renderer to follow the manual (B8, [D15](../DECISIONS.md#d15-hybrid-blender-baked-sprite--egui-painter-25d-rendering)) |
| Fader LED ladder (§4.1) | 11 segments | Manual text does not state the LED count (p.18, 23, 27 only say "LED graph"); the drawbar ladders are 8 LEDs (p.21) | `ASSUMED`; needs a panel-photo count ([control inventory](control-inventory.md)) |
| Split-point LEDs (§3, keybed) | 3 red LEDs | Manual p.15: green LEDs above the keyboard, plus L/M/H LEDs above the SPLIT ON/SET button | Spec wrong; inventory records both (`keybed.split_leds`, `split.leds`) |

---

## 5. Responsive Layout Modes (Plugin vs Standalone)

To keep every label legible on standard 1080p/1440p/4K monitors and inside DAW plugin windows:
1. **Full Panorama View (`1600×520` aspect, scalable `0.75x..2.0x`):** Shows the entire Nord Stage 4 top panel + 88-key interactive keyboard with split-zone color overlays and pitch/mod wheels.
2. **Focused Rack View (`1100×640` compact DAW mode):** Keeps the Top Master/Program/Morph/Scene bar + 88-key Split bar permanently visible while allowing large, zoomed-in inspection of `[ORGAN + PIANO]` or `[SYNTH + LAYER EFFECTS]` at full 1:1 readability.
   **Implemented (B12):** the overview scale-to-fits the window; the panorama is the `P` mode below, not a separate fixed-size layout. The rack view is the focus view (`Z`, `1`-`5`).

   | Key / gesture | Effect |
   |---|---|
   | `P` | Panorama on (viewport opens on the focused section, else a centred 2x window) / off |
   | strip drag or click | Centre the viewport on the pointer (`panel/panorama.rs`) |
   | wheel over the strip, `+` / `-` | Zoom the viewport about its centre (scale 0.25..1 of the panel; the egui zoom caps at 4x) |
   | arrows | Pan by 15 % of the viewport |
   | `Esc` | Leave the panorama (then focus, as before) |
   | `Z`, `1`-`5`, section header | Focus a section; clears the panorama |

   The strip (56 logical px high, panel aspect) sits under the detailed view and draws section outlines only. State is `ViewState::panorama: Option<Rect>` in logical panel px; the default view is unchanged. Not persisted: the app persists no view settings (no eframe storage), so a new session starts in the overview with the drawer closed. The library drawer slides in and out (`SidePanel::show_animated`), `L` toggles it; library logic is unchanged.
3. **Docked `.ns4b` Archive Browser & Knob Recall Mode:** The `.ns4b` library drawer docks as a resizable left `SidePanel` before the front-panel `CentralPanel` (never overlapping hardware controls), with an `Auto-close` toggle (enabled by default) that dismisses the browser upon selecting a Program, Piano, or Sample so the full top panel remains unobstructed. Several `.ns4b` banks can be open at once ([D239](../DECISIONS.md)): the first is the primary (Organize, Export, Live import), later ones merge into the Programs / Pianos / Samples lists; exact duplicates (same name, CRC-32, size) are skipped and different bytes under a taken name are listed as `<name> [<bank>]`. `Sync Knobs on Preset` defaults to `false` to match authentic non-motorized Nord Stage 4 hardware behavior (where OLEDs, `A:11..H:88` location, LED button rings, `fx_focus`, and layer/drawbar LED meters update while potentiometer caps keep their physical angle).
4. **No monetization UI (`ns4-s3pn`):** neither the standalone bar nor the plugin bar carries a sponsor, donate or purchase control, and the app opens no funding URL. The project stays strictly non-commercial under PolyForm Noncommercial 1.0.0 ([LEGAL.md](../LEGAL.md)).

## 5a. Double-stacked layout (design, issue #592)

**Status:** designed, not built. Bean `ns4-ojc9` implements it. Reporter and owner: issue #592 ("Gui Text") asks for larger panel text; the owner announced "a config option for a double stacked version, dividing controls in two sections horizontally". The text size option (5b) ships first and makes the print bigger by showing less of the plate; this layout makes the print bigger by showing more of it at the same size.

**Why.** The baked plate is 2632 x 650 px (4.05 : 1). In a 1920 px window it is drawn at 0.73x, so a 7.5 px label prints at 5.5 px. Stacking the controls in two rows halves the width the window must cover, so the same window draws everything about 1.4x to 2x bigger and nothing needs panning.

**Layout (plate px of `crates/nord-gui/assets/baked/panel_layout.json`).**

| Part | Source rectangle (x, y, w, h) | Content | Drawn |
|---|---|---|---|
| Row A (top) | 0, 0, 1323, 420 | wheels, `bay-master` (169), `bay-organ` (251), `bay-piano` (747), `bay-program` (995..1321) | 1.0x |
| Row B (middle) | 1323, 0, 1309, 420 | `bay-synth` (1325), `bay-fx` (1987..2569), right end cap | 1.0x, left-aligned under row A, 14 px of chassis fill on the right |
| Keybed (bottom) | 0, 420, 2632, 230 | 88 keys (`keys` y 425..643, white key 49 px) | uniform 0.503x (1323 / 2632): 1323 x 116, white key 24.6 px |

The split is at x = 1323, the 4 px gap between `bay-program` (ends 1321) and `bay-synth` (starts 1325): no control crosses it (measured 2026-11-24 on `panel_layout.json`: 0 control rectangles cross x = 1323 and all end at y <= 419; the test below keeps it true after the `ns4-80xe` rebake). Canvas 1323 x 956 (aspect 1.38 : 1). Gain over the wide plate for a window `W` x `H` (bar excluded): `min(W/1323, H/956) / min(W/2632, H/650)`; 1920 x 1000 gives 1.43x, 1760 x 1300 gives 1.99x. Order is reading order: Master, Organ, Piano, Program, then Synth, Effects, then the keys. ASSUMED: the Piano / Program row and the Synth / Effects row balance in width (1323 vs 1309 px); the split could instead follow the instrument's Layer A / B, which would move Program.

**How (least invasive).** The plate stays one picture and the overlays keep their normalized plate coordinates. A `PlateMap` replaces the single `plate_rect` that `baked_view.rs` and `baked_buttons.rs` derive every control rectangle from (`BakedRect::in_rect(plate_rect)`): it holds the three source rectangles above and their destination rectangles, paints the plate in three clipped pieces (`Painter::image` with a UV sub-rectangle each) and maps a normalized point to the piece that contains it. Wide mode is the identity `PlateMap` (one piece), so the default is byte-identical to today. Per piece, widgets already take their rectangle, so no widget changes.

| Part | Change |
|---|---|
| Setting | `layout` = `wide` (default) or `stacked` in `ns4/config.json` (`user_config.rs`), a bar pill `1x2` and `Ctrl L`, shared by the standalone and the plugin editor |
| Zoom (`baked_zoom.rs`) | the aspect becomes the canvas aspect; bay jumps use the bay's destination rectangle; `base` (5b) multiplies on top |
| Plugin window | `size_for_width` uses the stacked aspect (1.38 : 1) when the layout is stacked; the host window grows taller |
| Hover, section keys `1`-`5` | unchanged (bays resolve through the map) |
| Keybed hits and key lighting | `keys` rectangles map through the keybed piece |

Tests to ship with it: the split line crosses no control or bay rectangle (every `px` rect of `panel_layout.json` lies in exactly one source rectangle); wide mode produces the same rectangles as today for every control; stacked rectangles lie inside the canvas and do not overlap; headless frame at 100 and 150 percent text size shows no clipped label (the same check as `text_scale_tests.rs`); the plugin window aspect follows the layout; the config round-trips. Out of scope: a third arrangement (rack of bays in a column), rebaking art for the stacked layout.

## 5b. Text size option (issue #592)

`crates/nord-gui/src/text_scale.rs`, one setting for the standalone and the plugin editor: 100 to 200 percent in steps of 25 (ASSUMED, see the module doc).

| What | How |
|---|---|
| Choice | `TextScale` (percent, snapped); saved as `text_scale_percent` in `<config dir>/ns4/config.json` (`nord_core::user_config`), loaded once when a window opens, saved on every change, kept when the Nord OS update path is set or cleared (`firmware_setting::set` / `clear`) |
| Keys, bar | `Ctrl +` / `Ctrl =` larger, `Ctrl -` smaller, `Ctrl 0` 100 %; bar pills `A-` `NNN%` `A+` after the zoom pills (standalone bar and plugin bar) |
| Egui text | `scale_style`: every `TextStyle` size and the spacing fields (item spacing, button padding, interact size, icon sizes, indent, combo height, slider and text edit width) are the egui defaults times the factor, derived from the defaults each time so changes never compound; explicit font sizes of the chrome go through `px(base)` (thread-local factor, set by `apply` at the start of each frame) |
| Chrome geometry | bar height `bar_height()` (22 pt x factor), minimum standalone window `max(960, bar + window buttons)` wide and never wider than the monitor (`min_window_width`, `MinInnerSize`), minimum plugin editor width `max(640, bar + 40)` (`editor_min_width`), a size change or a saved size asks the host for the matching window (`size_window_for_text`, `EguiState::set_requested_size`; the host may refuse), Studio Settings viewport `880 x 520 x factor` (capped 1600 x 1000), dialog field widths via `px` |
| Plate | `BakedZoom::base` = factor: `Fit` (key `0`, pill `FIT`) shows the plate at `factor` times the fitted size, top-left first, and the usual pan / zoom / `1`-`5` keys move around. The zoom pills still read zoom against the whole plate (`150%`) |
| Plugin window | `size_for_width` adds the scaled bar height and the scaled minimum width, so the aspect lock keeps the plate fitted below a taller bar |
| Not used | egui's zoom factor: `egui_baseview` lays out with the native pixels-per-point, so a zoom factor mis-sizes the surface there (`editor/draw.rs` header); egui's own `Ctrl +/-` is switched off so it never doubles the scale |
| Screenshots | `--screenshot` ignores the saved size (always the plain 100 % panel) |

What it does not change: the printed legends are part of the baked plate picture; making them bigger means showing less of the plate (the magnified `Fit`), or the stacked layout (5a). Text drawn by egui over the plate (OLED text, tooltips of controls) follows the factor through the style.

Checks (headless, no window): `editor/text_scale_tests.rs` renders the real editor frame at 100 / 125 / 150 / 175 / 200 %: every bar label lies inside the window and its clip rectangle and no two overlap; the `DEBUG` pill text is 10 / 16 / 21 pt high at 100 / 150 / 200 % (EMPIRICAL text-row measurement, not a bitmap); the Studio Settings dialog (all five tabs) fits inside the editor window and its title grows with the size at 100 / 150 / 200 %; an 800 point wide host is asked for a window the bar fits in at 150 and 200 %; Ctrl + / - is ignored while a text field has the keyboard (`text_scale.rs`); the Studio Settings theme spacing follows the size (`debug_dialog.rs`); a malformed `text_scale_percent` reads as absent and keeps the other saved settings (`user_config.rs`); every check is on painted text rectangles, there is no golden bitmap (the headless context has no GPU); `Ctrl +` / `-` / `0` change and save the size; the plate is magnified by the factor and starts at the left; `size_for_width` keeps the aspect under the taller bar. `panel/baked_zoom_base_tests.rs` checks `BakedZoom::base`. Not verified here: a real display and a DAW host.

---

## 6. Baked sprite proof (B7, Phase 6b)

First measured output of the D15 baker: one knob filmstrip and one fader cap with a separate shadow, rendered by `assets/blender/build_proof.py`, committed under `assets/panel/proof/` with SHA-256 hashes in `manifest.json` (`python tools/panel_assets.py verify`). Details and sizes: [panel-blender-proof.md](../research/panel-blender-proof.md).

| Item | Value | Tag |
|---|---|---|
| Knob filmstrip | 64 frames, 8x8 grid, frame 96 px (1x) / 192 px (2x), pointer -135 to +135 deg clockwise | rotation `EMPIRICAL` (rotary scale on reference photo, measurement only) |
| Fader cap | 96 px / 192 px sprite, separate shadow sprite of the same size | `ASSUMED` geometry (original) |
| Light | one top-left sun, elevation about 55 deg | `ASSUMED`, D15 |
| Camera | orthographic, 14 deg tilt toward player | `ASSUMED` |
| Size, one of each asset | 420 KB (1x), 1.28 MB (2x) PNG | measured |
| Full-panel estimate (4 knob styles, 3 fader styles) | 1.56 MB (1x), 4.62 MB (2x) PNG | counts `ASSUMED` until B2 |

## Label legibility rules (B8)

Printed text never goes below 7.5 px (`widgets::MIN_FONT_PX`) and must not touch anything it does
not describe. The headless audit in `crates/nord-gui/src/panel/legibility_*.rs` enforces it per
rule (overlap and 0.9 px gap, control bezel, clipping, section edge, group frame, knob ring; table
in [GUI testing](../gui-testing.md)). Layout decisions it led to:

- Rotary Speaker box: the frame extends 52 px left of the button column (the Master area is empty
  there) and 30 px right, capped 3 px inside the Master section. Wide legends (`SLOW/STOP - FAST`,
  title) are centred on the frame, narrow ones on their button. The `rotary.speed_leds` rectangle
  (`ESTIMATED`) is widened from 20 to 33 photo px so the row holds `LED SLOW   LED FAST`.
- Shift legends place their state LED after the measured text, not at a character-count guess.
- Effects captions start 6.5 px below the control (clear of knob shadow and button bezel) with
  8.5 px between stacked lines.
- The Synth ON button prints SOLO to its right (the OLED sits right below it); the OLED soft
  labels use two 9.5 px lines and are kept inside the glass; the vibrato selector no longer repeats
  the VIBRATO heading (SELECT).
- Two-word percussion names (`VOLUME SOFT`, `HARMONIC 3RD`) stack when the neighbour is closer
  than the full name.

### Round 2: fader caps, LEDs, stacked legends, comparison with the photos

Added rules `F`, `E`, `T`, `V`, `M` ([GUI testing](../gui-testing.md)). Changes: drawbar footage
numbers print 13 px below the slot (the cap and its shadow reach 6 px past it); state LEDs of Shift
legends sit before the text (photo: `o SUSTPED`, `o CLOSE MIC`); stacked lines use 9.5 px pitch;
the Program buttons 5-8 and MON/COPY moved 3 px down, the Piano zone LEDs 3 px down, the split LEDs
2 px up (`layout_tweaks.rs`; control rectangles in `nord-panel` are unchanged); the layer-strip
`FX FOCUS` LED now has its caption (left of the LED, over the free strip above the faders); the
Effects amp RES trimmer has a `RES` name to its right.

### Round 3: functional identification (titles, frames, scales, names)

A user cannot find or understand a control without its name, frame or scale, so round 3 prints what the real panel prints and the emulator omitted. Frames come from `nord_panel::layout::frames` ([control inventory](control-inventory.md) section 7), painted by `panel/frames.rs` before the controls of each bay.

| Section | Added |
|---|---|
| Piano | `ACOUSTICS`, `UNISON`, `KB TOUCH`, `DYN COMP` titles; an always-visible `PIANO SELECT` box with the six type names beside a 2 x 3 LED block (left column ELECTRIC, UPRIGHT, GRAND; right column CLAV, DIGITAL, MISC; `EMPIRICAL`, read from `NS4_HA88_Piano-01`); TIMBRE listed BRIGHT, MID, SOFT top to bottom (was SOFT, MID, BRIGHT) with DYNO1 / DYNO2 in a second column (dimmed unless the Electric type is selected) |
| Organ | `ORGAN MODEL`, `VIB/CHORUS`, `B3 PERCUSSION` frames (the former `ORGAN MODEL` / `VIB/CHORUS` text tags are the frame titles now); per-drawbar register names over each ladder (manual p.19: BASS16, STR16, FLUTE8, OBOE8, TRMP8, STR8, FLUTE4, STR4, 2 2/3) and the numerals 1-8 beside every LED ladder (the ladder LEDs are 1 px smaller in radius to make room) |
| Synth | `MODE`, `ARPEGGIATOR / GATE`, `VOICE`, `VIBRATO`, `LFO`, `OSCILLATORS`, `FILTER`, `AMP`, `UNISON` boxes (the old heading texts are the box titles; the MODE, VOICE and UNISON button legends became SELECT / MODE / SELECT so a title is not printed twice); `0` and `10` (`-10` and `10` for the oscillator ENV AMT) at the two ends of every pot's tick ring (`widgets/scale.rs`); `INFO`, `LIST`, `LIST` under the three OLED dials |
| Layer Effects | `MOD 1`, `MOD 2`, `AMP SIM/EQ`, `DELAY`, `COMP`, `REVERB` boxes; `0` / `10` scale ends on every knob, `-15` / `15` under the three EQ knobs (no room beside them) |
| Program | `MORPH ASSIGN`, `SPLIT`, `MST CLK`, `TRANSP`, `PRESET LIBRARY`, `PROGRAM` boxes on the program plate (the SPLIT, MST CLK and TRANSP button legends are ON/SET, TAP/SET, ON/SET so the title is not repeated) |
| Keybed | `L`, `M`, `H` printed beside the three split-point LEDs |
| Contrast | every dimmed legend (ZONE -/+, PEDAL TAP, SOLO, Shift legends of unsupported controls, dimmed button names) goes through `style::dim_legend`, which fades the ink but keeps a WCAG contrast of at least 4.5:1 against the slate and the red (grey 150 on red was 3.9:1, grey 105 on slate 2.2:1) |

Room for the boxes comes from `GROUP_TWEAKS` in `layout_tweaks.rs` (rows moved by 6 to 30 px so a 7.5 px legend, a border and a title fit between stacked boxes); the photo tables stay untouched. A few controls cannot sit exactly where the photo has them: the three OLED dials moved up 10 px, the Vibrato box right 5 px, the Piano TIMBRE button left 9 px.

### Round 4: numerals and variation lists

| Section | Added |
|---|---|
| Synth, Layer Effects, Rotary | numerals on the radial line of every tick of the knob ring (`widgets/scale.rs`): `0 ... 10` on pots, `-15 ... 15` on the EQ knobs, `-10 ... 10` on the oscillator ENV AMT, `200 250 400 600 1K 2K 4K 6K 8K` on the amp FREQ knob (`EMPIRICAL`, photos `NS4-Panel-synth`, `-effects`, `-organ`); Master level, the Piano dial, the OLED dials and the RES trimmer print none. A scaled knob body is drawn 3 px smaller (`ASSUMED`, least that keeps `10` and `-10` of neighbours apart); names moved below the `0` / `10` numerals (`KNOB_CAPTION_DROP`, `pots::NAME_DROP`) |
| Layer Effects | variation lists (`bay_effects/lists.rs`): Mod 1 RM / TREM / A-PAN beside A-WAH / WAH / PUMP, Mod 2 CHOR / FLANG / PHAS beside VIBE / ENS / SPIN, Amp SMALL / JC / TWIN, Delay CHOR / VIBE / ENS beside FLAM / SPACE and HP / LP beside BP, Reverb ROOM / BOOTH / SPRING beside STAGE / HALL / CATH, BRIGHT / DARK (names `VERIFIED` against the manual p.49-53; LED order is the cycle order of `ui_state/fx/tables.rs`); the FLT and ROT buttons sit beside the amp model selector |
| Piano | `1 2 3` under the UNISON and DYN COMP LEDs, `H M L` under KB TOUCH (full names do not fit 13 px) |
| Program | the PROGRAM box has a 2 px left pad so the PAGE / BANK buttons sit inside its border |

### Round 5: LED names and lists

| Section | Added |
|---|---|
| Synth | MODE box prints `SAMPLES` / `ANALOG` and VOICE box `MONO` / `LEGATO`, each with its LED above the button (`bay_synth/names.rs`; the lit LED follows the layer state, POLY lights none; the old SELECT / MODE legends are gone, the button keeps its value). The MODE frame grew 12 px at the top for the two rows |
| Piano | KB TOUCH prints `HEAVY`, `MED`, `LIGHT` under its LEDs on two staggered lines (the middle name lower; the names are wider than the 13 px LED pitch); the PIANO SELECT box moved 3 px lower (nudge 45 to 48 for type and info, the dial and LIST stay at 45) |
| Layer Effects | the amp list has a second column `TO ROTARY` / `LP FILTER` / `HP FILTER` with LEDs that follow the cabinet choice (TO ROTARY dimmed with its reason on Organ layers); the small buttons beside the selector keep working and lost their FLT / ROT text |
| Program | the PROGRAM title sits at 6 % of the frame width instead of on its left corner |

Remaining after round 5: the amp FLT / ROT buttons are unlabeled pills (the names are in the list above them).

### Round 6: VIB/CHORUS type names, Rotary tab, Undo item name

The `C2 V3 C3` / `V2 C1 V1` matrix is not a Synth control: it sits in the **Organ** VIB/CHORUS box (photo `NS4_Panel-crop-left.jpg`), next to the type button, and it was already wired (`organx_scanner_{n}`, `OrganEdit::Scanner`). The Synth VIBRATO box (WHL / DLY / ON, A.T. / PED) is a separate, already inventoried control set. The LEDs were drawn unlabeled above the button in the wrong order.

| Item | Change |
|---|---|
| VIB/CHORUS matrix | `panel/organ_vib_matrix.rs`: two rows of three LEDs right of the type button with the names printed above (`C2 V3 C3`) and below (`V2 C1 V1`). The selector order is now `V1, C1, V2, C2, V3, C3` (`VERIFIED`: the stored `VibratoTypeB3` order, `cbin-and-presets.md`; the printed matrix is that sequence as a snake from the bottom right). Before, the button stepped V1, V2, V3, C1, C2, C3 and the LEDs sat in that raster. 7.5 px names need 11.5 px per column, so the VIB/CHORUS frame grew 23 px to the right (pad 26) and the B3 PERCUSSION group moved 3 px right, its frame 4 px less padding on the left; the A / B buttons moved 5 px down so their names clear the lower row |
| Rotary tab | `bay_master/rotary.rs`: `ROTARY SPEAKER` is printed in dark ink on a light blue-grey tab (`MODULE_HEADER_BAR`) that spans the top of the box with rounded upper corners (`VERIFIED` against the photo; the photo wraps the title in two lines, ours stays one line at 13 px tab height) |
| Undo | `Undo to before` / `<operation>?`: the item is the name of the engine's last undoable operation (`UndoOp::label`: Load Preset, Synth Group, FX Group, Global FX, All FX Off, Clear Morph, Paste, Synth Init, Layer Init, Program Change) from `PanelTelemetry::undo_label`. `VERIFIED`: the dialog texts `Undo to before` / `%s?` / `Ok` / `Redo` (research s15.4, s17.1). `ASSUMED`: the item strings are the manual p.43 operation names; the firmware's own item strings were not reached |

Audit: 0 before, 10 on the first run with the matrix names (2 E, 2 G, 6 L), 4 after widening the frame (L: names 9 px apart), 0 after the 11.5 px pitch; allow-list empty.

Remaining differences against `.reference/crops` (kept: legibility over fidelity, or out of scope):

| Section | Photo | Emulator |
|---|---|---|
| Organ | bold caps, `C2 V3 C3` matrix | regular text, LED column per model |
| Piano | numbered LEDs (1 2 3) on UNISON / DYN COMP, SOFT REL / STR RES names beside LEDs | LED rows under the buttons, names inside the buttons |
| Synth | `MODE` box holds SAMPLES / ANALOG LEDs, VOICE box MONO / LEGATO LEDs, 1-9 scale numerals all around each knob | one button per choice with its current value, scale ends only |
| Effects | variation lists beside the rocker LEDs, FREQ / RES numerals (200 ... 8K), FX FOCUS column inside a box | one LED row per selector without names, no numerals on the amp FREQ knob |
| Master / Program | the PAGE / BANK buttons inside the PROGRAM box | the PAGE buttons touch its left edge |
| Keybed | keys only | keys plus the three LED names |

### Round 7: LED spacing, Rotary Speaker box, ORGAN MODEL order

LED spacing rules (audit rules `B` and `D`, `panel/legibility_leds.rs`), values in px at the panel scale (`LAYOUT_SCALE` 2):

| Pair | Minimum gap between bezels | Status |
|---|---|---|
| LED to the bezel of a button (rectangle + 1.5 px; Effects pills use a 2 px bezel and are built 0.5 px further out) | 2.5 | ASSUMED: the photos show a visible dark gap, none is measurable at 2000 px; chosen to match rule `C` |
| LED to LED (centre pitch >= diameter + 2) | 2.0 | ASSUMED: same reason |
| Bar to bar and LED to bar in level ladders | 1.5 | EMPIRICAL: the photos (`NS4-Panel-organ`) show the 11-segment ladders with a thin dark line between bars; 1.5 px is the smallest gap that reads as a separate bar |

First run: 53 `B` and 21 `D` (plus 40 ladder bars at 1.0 px, which the first version of the rule did not record); 0 after. Fixes live in shared helpers: `ctl::led_above` / `LED_LIFT` (state LED above a gesture button), `ctl::LEGEND_DROP` (Shift legend with its LED), `bay_effects::ctx::LED_LIFT_FX`, `lists::COL_DX` / `BOTTOM_ROW` / `NAME_GAP`, `piano_widgets::led_strip` (radius follows the pitch), `bay_piano_select::SIDE_LED` / `LED_ROW_DROP`, `organ_widgets` / `piano_widgets` `SEG_GAP`, `organ_vib_matrix::PITCH_Y`. Names printed above a button with an LED use `ctl::TAG_LIFT_LED`.

Rotary Speaker box (photo `NS4_Panel-crop-left.jpg`): every control and legend sits on the frame's vertical centre line (`layout_tweaks`: the `rotary.` controls move 11 px left; the frame is symmetric, 41 px each side). ORGAN and STOP MODE print as "o NAME" above their button (`ctl_led_tag::tag_led`: text centred on the button, LED left of the text, as on the instrument); SLOW / FAST is one row centred on the line (`bay_master/rotary_speed.rs`); the button pitch is 34 px so the legend above, the 2 px stacked gap (rule `V`) and the name line fit. DRIVE moved 3 px up so its name clears ORGAN.

ORGAN MODEL order (VERIFIED, presentation only, photos `NS4_Panel-crop-left.jpg`, `NS4_HA88_Organ-01_241008.jpg`, manual p.18): the printed columns read FARF, VOX, B3 (left) and PIPE1, PIPE2, B3 BASS (right). `organ_model::SLOTS` maps each printed slot to the engine index; `organ_logic::MODELS` (B3, B3 Bass, Vox, Farf, Pipe 1, Pipe 2) and the button's step order are unchanged. Other lists checked against the photos and found in order: PIANO SELECT (ELECTRIC / CLAV, UPRIGHT / DIGITAL, GRAND / MISC), TIMBRE, VIB/CHORUS matrix (C2 V3 C3 / V2 C1 V1), Mod 1, Mod 2, Amp, Delay variation and filter, Reverb lists, Synth MODE and VOICE.

### Round 8: visual details measured on the straight-on HA88 render (`fix/visual-details`)

Measured on `NS4_HA88_TopDown-01_241008.jpg` (0.1195 mm/px both axes, key pitch 197.7 px = 23.63 mm).

| Item | Photo | GUI before | Change | Status |
|---|---|---|---|---|
| Knob numeral ring (OSC CTRL: `5` and `0`/`10`) | 8.2 mm / 8.7 mm from the knob centre (body 6.3 mm) | 10.7 mm, plus a drawn tick ring the photo does not have | no ticks on knobs that print numerals, ring 10.4 mm (`widgets::scale`, `GAP` 4.8 px) | `EMPIRICAL`: the 7.5 px legibility minimum is about 1.4x the photo's digit height, so a ring tighter than 10.4 mm makes neighbours collide (GAP 3.6 leaves 5 collisions, 0.8 leaves 91); the rest of the gap to 8.5 mm cannot close without smaller type |
| Black key width | 130 px outline = 0.66 of the pitch (the flat dark top face alone is 0.45) | 0.64 | kept; `black_key_width_matches_the_render` pins it | `VERIFIED` (+-0.05); black key length measures 0.67 (code 0.62, `ASSUMED`, not changed) |
| Organ A/B vibrato buttons | one wide ON button on the hardware; the emulator splits it per layer | 7.6 mm wide at 8.0 mm pitch (0.4 mm gap, bezels touched) | 6.6 mm wide, 1.4 mm gap (`layout/organ.rs`, test `vibrato_buttons_keep_a_gap`) | `EMPIRICAL` |
| Unlabelled green dots under the Synth / Piano / Organ faders | `AUX KB` legend left of each LED (`NS4-Panel-synth` crop) | dots without a name | `aux_flag::aux_legend` prints `AUX KB` for all three strips | `VERIFIED` against the crop |

Still unlabelled: the layer focus LEDs above the A/B/C pads (hardware: amber ON/OFF LED with an `ON/OFF` legend); Synth VIBRATO name lists and the COMP ACTIVE / BRIGHT-DARK ids are not done. `NUMERAL_REACH` of the legibility audit grew from 16 to 20 px (the probe rectangle no longer includes a 5.5 px tick ring). `legibility_allow.txt` stays empty.

## Section headers (GUI styling, 2026-11-19)

Status: accepted (owner request: production-ready look, compared with the Nord editor skin; no art copied). Organ, Piano, Synth, Layer Effects and Program headers use a graphite rim and tab (`style::HEADER_DARK`, edge `HEADER_EDGE`, ink `HEADER_INK`, 13:1) instead of the photographed silver strip, which gave the FX FOCUS and LAYER FX captions about 1.5:1. Synth gets a tab (`SYNTH_TAB_MM`) so ON, its LED and SOLO sit on a plate; the hardware prints them on the red panel (not photo parity). The ON LED sits left of its button inside the tab. The Rotary Speaker tab keeps the verified silver plate.

---

## 9. Offline HTML/CSS-to-`egui` Baked Panel Pipeline (D15 Amendment, `ns4-op1w`)

To render the approved ND Matrix 1 HTML/CSS design (`prototypes/panel-mock/`, Slate Matrix identity, D251) inside OpenGL `egui` hosts without embedding a WebView:

- **Offline Baker (`tools/bake_panel_from_html.py`):** Launches headless Chromium or Edge (DevTools protocol) at `2x` device scale, hides dynamic moving elements via `.bake-static`, and exports deterministic assets to `crates/nord-gui/assets/baked/`:
  - `panel_plate_2x.png`: static graphite chassis, side cheeks, chamfered section headers with accent underlines, silkscreen, scales, keys and the `ND MATRIX 1` wordmark.
  - `panel_sprites_2x.png` + `panel_sprites.json`: tile buttons (`black`/`grey`/`accent`, `up`/`down`), hexagonal knob caps and round encoder caps with 14 px shadow padding, fader cap, sand/navy ribbed drawbars, ribbed mod wheel drum and pitch stick.
  - `panel_layout.json`: normalized `(x, y, w, h)` bounding boxes for all annotated `data-control-id` controls, LEDs, OLEDs, and 88 keys.
- **Runtime Loader & View (`crates/nord-gui/src/baked.rs`, `crates/nord-gui/src/panel/baked_view.rs`):**
  - `BakedPanelAssets::get_or_load` uploads the baked atlas and splits `panel_plate_2x.png` into `<= 2048px` horizontal GPU texture tiles so OpenGL contexts with `max_texture_side = 2048` never fail.
  - Zoom and pan (D240, `panel/baked_zoom.rs`): `render_baked_panel` takes the plate rectangle from `BakedZoom` (zoom 1 = whole plate fitted, up to 6) and clips to the viewport. Keys `1`-`5` jump to the `bay-organ`, `bay-piano`, `bay-program`, `bay-synth`, `bay-fx` rectangles, `0` / `Esc` fit, `+` / `-` step; Ctrl + scroll zooms at the pointer, dragging empty plate pans. Bar pills are shared by the standalone and plugin bars. Commands: `docs/guides/commands.md`.
  - No photo texture ships: the former embedded `panel_texture.jpg` (a vendor photo) is removed (D16); all panel pixels come from the baked plate and sprite atlas, and chrome colours from `style.rs` tokens. `tools/check_no_vendor_photos.py` enforces it in CI.
  - `render_baked_panel` renders the tiled plate, interactive `pitch_stick`, `mod_wheel`, rotated knobs, buttons, faders, 9 drawbars, live LEDs, live `main` and `synth` OLEDs, and the 88-key keybed wired to `AudioHandle::send_keyboard_midi`.
  - Controlled by `Stage4PanelState::use_baked_plate` (enabled in the standalone `nd-matrix-1` binary; `false` by default in `Stage4PanelState::default()` so existing procedural layout and legibility tests remain valid).

## 10. Naming and trademark rule for user-facing text (D16 follow-up)

Our product name is **ND Matrix 1**. "Nord", "Nord Stage 4" and "Clavia" appear in user-facing text only descriptively ("backup files for the Nord Stage 4", "Nord OS update"), never as the name of our window, panel, library, installer or plugin.

| Surface | Text | Status |
|---|---|---|
| Window class (`eframe::run_native`) and default program title | `ND Matrix 1` (`nord-gui` `branding::PRODUCT_NAME`) | rule |
| Procedural chassis wordmark | `nd matrix 1` (`branding::CHASSIS_WORDMARK`) | rule |
| Library drawer heading | `Backup & Sample Library (Nord Stage 4 files)` | descriptive |
| File dialog filter (backups, sounds) | `Backup and sound files for Nord Stage 4` | descriptive |
| Plugin name, installers, `.desktop` | `ND Matrix 1` | already compliant |
| Shipped plugin bundle files | `nd-matrix-1.clap`, `nd-matrix-1.vst3` (`bundler.toml`; legacy `nord-plugin.*` removed by installers) | rule |

Unchanged on purpose: crate and binary names (`nord-*`), host parameter IDs and CLAP/VST3 ids (saved DAW projects), data matches such as `prog.name == "Nord Stage 4 Program"` (written by the hardware), hardware text mirrored on the OLED (`Nord Sample Library 4`), pedal model names, and official download links. Pinned by tests in `crates/nord-gui/src/branding.rs` and `file_picker.rs`.

"Nord OS update" and "Nord OS update file" stay as labels because they name the vendor's firmware file the user must supply (nominative use), not our product.
