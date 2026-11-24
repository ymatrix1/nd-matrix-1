# Running the GUI and testing firmware paths

The standalone `nd-matrix-1` (crate `nord-gui`) plays through cpal and midir. Every opt-in firmware
engine path of `ns4 render-patch` can be chosen live from dropdowns in the bar above the panel, or
at start-up from the same CLI flags. Code: `crates/nord-gui/src/firmware_panel/`, shared option
struct `nord_core::render_opts::FirmwareOptions`.

## Commands

```bash
# Build and run (release is needed for real-time audio)
cargo run --release -p nord-gui --bin nd-matrix-1
# Everything verified, on from the start
cargo run --release -p nord-gui --bin nd-matrix-1 -- --firmware-all
# Pick paths one by one (same flags as `ns4 render-patch`)
cargo run --release -p nord-gui --bin nd-matrix-1 -- --tone-source b3wheels --b3-tail off --synth-chain strip --filter-engine firmware --firmware-fx --firmware-piano-level --measured-laws
# Load a backup and a piano library (large files live in .reference/, git-ignored)
cargo run --release -p nord-gui --bin nd-matrix-1 -- ".reference/Nord Stage 4 Factory Restore RevK v1.66.ns4b" .reference/piano.npno
```

Positional arguments are routed by extension (`.ns4b`, `.ns4p`, `.npno`, `.nsmp4`); `--backup`,
`--program`, `--piano`, `--sample` also work, and files can be dropped on the window.

Large files live in `.reference/` of the **main checkout** (`C:\dev\nord-stage-4-emulator\.reference\`),
not in a worktree: pass the absolute path from a worktree, e.g.
`"C:\dev\nord-stage-4-emulator\.reference\Nord Stage 4 Factory Restore RevK v1.66.ns4b"`.

## The bar

Whole-engine presets `Firmware preset` (`--firmware-all`), `Classic preset` (`--legacy-all` plus
the engine amp) and `Defaults`; each group (Organ, Piano, Synth, FX) also has small `C` (Classic)
and `F` (Firmware) buttons that set only that engine's controls and leave the others as picked
(`firmware_panel/presets.rs`; tests check that the four engine presets compose to the whole
presets). Below, one row per group, each switch with a tooltip (`[STATUS] flag` and a one-line
meaning) and an `Engine: ...` status line with the active set (also in the window title). Every
opt-in field of `FirmwareOptions` has a control:

| Group | Controls |
|---|---|
| Organ | Source, B3 tail (enabled with B3 wheels), B3 Bass processor, Farfisa processor, Rotary drive, Level, Leslie (engine / firmware), Ring and Leslie gain (enabled with the firmware Leslie) |
| Piano | Level, SMP bus scale, FX path scale, Layer law |
| Global | Firmware routing |
| Synth | Chain, Stage oscillators, P table, Filter, Measured laws, Shape (checkbox + slider), Sample kernel, Sample pitch |
| FX | Amp path, All ports, then one checkbox per port: Delay laws, Delay time, Reverb mix, Reverb (Spring and the multi-tap types 2..11), Compressor, Mod 1, Mod 2, Amp EQ |

Changes are sent as `AudioCommand::SetFirmwareOptions` and `FirmwareOptions::apply_exact` runs on
the audio thread between blocks (plain field writes, no allocation; rotary scaling is a flag, the
program's drive is never rewritten). Two switches allocate once: a real B3 tail change rebuilds
the wheel tables (resets the wheel phases and can click), and switching the firmware Leslie on
or changing its ring builds a new processor (the rotor restarts); identical repeated commands keep
both. Switching the Leslie back to the engine rotary drops the processor.

## Headless check (no window, no audio device)

```bash
nd-matrix-1 --firmware-all --self-test out.wav   # renders organ+piano+synth chord for 1 s
nd-matrix-1 --measured-laws --dump-options       # prints the active set
```

The self-test prints peak, RMS and the RMS difference to the default engine (0 for defaults) and
exits non-zero on non-finite samples. The window itself needs a display and an audio device.

## Playing with the computer keyboard

The GUI plays from the PC keyboard (group "Computer keyboard" in the bar under the panel, default on). Keys are physical positions, so the same on QWERTY and ABNT2.

| Keys | Action |
|---|---|
| `A S D F G H J K L ; '` | white keys C D E F G A B C D E F from the base octave (default C4 = MIDI 60) |
| `W E T Y U O P` | black keys C# D# F# G# A# C# D# |
| `Z` / `X` | base octave down / up (C0..C7) |
| `C` / `V` | velocity down / up by 8 (1..127, default 88 like a mouse click) |
| `Space` | sustain pedal (CC 64 down while held) |
| `Shift` + `P` / `Z` / `L` | panorama, focus zoom, library drawer while the keyboard is on (plain keys play) |

Key down is a Note On, key up a Note Off; auto-repeat is ignored. Notes go through the keyboard path like the on-screen keybed (Local Control Off silences them, the MIDI-in thread is not used) and light the on-screen keys. Everything is released when the window loses focus, a text field (library search) has the keyboard, or the checkbox is cleared. Visual check: run `nd-matrix-1`, hold `A` and `G` (two lit keys at C4 and G4), press `X` then `A` (C5), tap `Space` with a piano program to hear the sustain, click the bar's Velocity slider to change the dynamics. Tests: `panel/computer_keys.rs` (map), `panel/computer_kb.rs` (state machine), `panel/computer_kb_tests.rs` (headless key events through the full panel, commands in the audio queue). Decision D80.

MIDI input: the bar under the panel also has a "MIDI input" group (port combo box, Rescan button, status text). At start-up the first port is opened as before; pick another port or "Off", or plug a keyboard in and press Rescan (hot plug; a replaced or removed port sends All Notes Off and clears the lit keys). A MIDI keyboard plays with its own velocity (the Note On velocity is passed to the engine unchanged); only the mouse (88) and the computer keyboard (slider, default 88) use a fixed value. Code: `audio/midi_ports.rs` (list, choice, GUI side), `audio/io.rs` (MIDI thread); tests in both files.

## Screenshots, focus view and the legibility check

```bash
# One PNG per section at 2x (builds once, writes target/shots/<section>.png)
python tools/gui_shots.py [--out DIR] [--scale 2] [--sections organ,synth] [--no-build]
# One shot by hand; sections: organ piano program synth effects master keybed (default: whole panel)
nd-matrix-1 --screenshot out.png --screenshot-section synth --screenshot-scale 2
```

`--screenshot-scale N` is physical px per logical panel px. The window opens off-screen without
taking focus or a taskbar entry, is resized to the region, the frame is cropped to it, and the
process exits 0 after saving (`std::process::exit`; the normal close path ends with
`0xcfffffff` on Windows). A 20 s watchdog exits 3 when no PNG arrives; `gui_shots.py` also kills
a child that runs over 30 s. A whole-panel shot is limited by the OS window size (the keybed
is capped at 1x for that reason). F12 still saves the live window to `screenshot.png`. Code:
`crates/nord-gui/src/screenshot.rs`.

Focus view and panorama below apply to the non-baked panel only. The baked plate (default) zooms with `panel/baked_zoom.rs`: keys `1`-`5`, `0` / `Esc`, `+` / `-`, Ctrl + scroll, drag to pan, bar pills; see [Commands and zoom](guides/commands.md).

Focus view (B12): click a section header or press `Z` (`Shift+Z` while the Computer keyboard is on; section under the pointer, else the last
one) to zoom that section to fill the window (up to 4x, crisp text: the egui zoom factor, not a
bitmap scale); `Esc` returns, `1`-`5` jump to Organ, Piano, Program, Synth, Layer Effects. In the
overview the window scale-to-fits the panel. Code: `crates/nord-gui/src/panel/view.rs`.

Panorama (B12): `P` (`Shift+P` while the Computer keyboard is on) shows a pannable / zoomable part of the panel with a strip of the whole
instrument under it (drag or click the strip, wheel or `+` / `-` zoom, arrows pan, `Esc` leaves);
`L` (`Shift+L` while the Computer keyboard is on) toggles the library drawer. `--screenshot` never enters the panorama and never draws the
drawer, so `gui_shots.py` output is unchanged. Code: `panel/panorama.rs`, tests
`panel/panorama_tests.rs`.

Scale: the panel is drawn at `LAYOUT_SCALE` = 2 (3120 x 840 logical px, `panel/layout_px.rs`) so
that 7.5 px labels fit between neighbours. A 1900 px window therefore shows the overview at about
0.6x: read labels in the focus view or in `gui_shots.py` output.

`panel/legibility_tests.rs` (collector, control rectangles) and `panel/legibility_rules.rs`
(geometry rules) render the real panel headless and record every painted text galley with its
clip rectangle, plus the group frames and knob extents the widgets report through the test-only
`widgets/probe.rs`. The audit fails on:

| Rule | Meaning |
|---|---|
| min font | any text under `MIN_FONT_PX` (7.5 px) |
| `L` | two labels overlap or are closer than 0.9 px |
| `C` | a label overlaps (bezel +1.5 px) a control rectangle whose body it is not printed on |
| `K` | a label is clipped by the clip rectangle it was painted with (OLED soft labels) |
| `S` | a label crosses the edge of a section rectangle (text cut by the section) |
| `G` | a label crosses a group frame (Rotary Speaker box, `group_frame`) other than its own title |
| `N` | a label touches a knob body or its tick ring (the ring is open below the knob) |

Round 2 (`panel/legibility_caps.rs`, same collector plus `Kind::CapTravel` / `Kind::Led` records
from the widgets) adds:

| Rule | Meaning |
|---|---|
| `F` | a label overlaps the travel of a fader cap (drawbar numbers hidden by the pushed-out cap) |
| `E` | an LED bezel is closer than 4 px to a group frame edge, or on it |
| `T` | a label comes closer than 0.6 px to an LED bezel |
| `V` | two stacked labels (sharing a third of their width) have less than 2 px between their ink; text inside OLEDs is exempt |
| `M` | a knob's name is more than 12 px from its tick ring (or missing); a caption beside a trimmer counts |

The bezel margin of rule `C` grew from 1.5 to 2.5 px (the button bezel is 2 px). Stacked-line
pitches (`ctl::TAG_LINE`, `bay_effects::ctx::CAPTION_LINE`, Synth headings, Piano TIMBRE rows)
are 9.5-10 px so that rule `V` holds; state LEDs of Shift legends sit before the text, as printed
on the instrument.

Round 3 adds frames (`panel/frames.rs`, `widgets/scale.rs`) and needs no new rule: boxes record themselves with the widget probe, so rule `G` keeps every label inside or clear of a box, rule `E` keeps LEDs 4 px from its edge, rule `C` keeps scale numerals off control rectangles and rule `N` off tick rings. Unit tests cover the frame table (`nord-panel`: members exist, frames inside sections; `nord-gui`: boxes do not overlap after nudges), the numerals, the type and timbre names, the keybed LED letters and the contrast floor (`style::dim_legend`, 4.5:1 against slate and red).

Round 4 adds knob numerals (`widgets/scale.rs`) and the Effects variation lists (`bay_effects/lists.rs`) with no new rule. Rule `C` exempts a numeral printed around the tick ring of its own knob (`is_own_numeral`: the text is one of the knob's numerals and lies within 16 px of its probe rectangle). First run with the numerals on 116 collisions (61 C, 44 L, 10 G, 1 V), 0 after names moved below the `0` / `10`, knob bodies shrank, frame pads grew and a few nudges; the lists and piano numerals gave 12 C, 2 E, 2 L, 2 T and 6 G / 4 V respectively, all fixed.

Round 6 adds the VIB/CHORUS type names (`panel/organ_vib_matrix.rs`) and the Rotary Speaker tab with no new rule; the first run gave 2 E, 2 G, 6 L (frame too narrow, names 8 px apart, A / B names under the lower row), 0 after the 11.5 px column pitch, a wider frame, the A / B nudge and the percussion box 3 px right.

Round 5 adds LED names (`bay_synth/names.rs`), KB TOUCH full names and the amp second list column with no new rule; first run 1 L (MED against the PIANO SELECT title), then 1 S (LIST at the section edge) while nudging, 0 after.

Round 7 (`panel/legibility_leds.rs`, LED spacing; records `Kind::Led` and the ladder bars `Kind::LedSeg`) adds:

| Rule | Meaning |
|---|---|
| `B` | an LED bezel is closer than 2.5 px to a button bezel (button layout rectangle + 1.5 px); LEDs whose centre lies on the button are exempt |
| `D` | two LED bezels closer than 2 px (centre pitch under diameter + 2), or two ladder bars / an LED and a ladder bar closer than 1.5 px (2 px for LED to bar) |

First run 53 `B` + 21 `D` (+ 40 bars at the old 1.0 px gap), 0 after; the allow-list stays empty. Shared fixes: `ctl::led_above` / `LED_LIFT`, `ctl::TAG_LIFT_LED`, `ctl_led_tag::tag_led`, `SEG_GAP` of the ladders, list pitch `COL_DX` and `NAME_GAP` of the Effects lists, plus nudges in `layout_tweaks.rs`.

`legibility_allow.txt` lists tolerated violations (empty now; the test also fails on stale
entries). Run with `NS4_WRITE_COLLISIONS=<file> cargo test -p nord-gui collisions_match` to dump
the current list. Fixes go into the shared helpers first: `widgets::label_fit` / `fit_size`
shrink a text to the room it has (never below 7.5 px), `ctl::tag_fit` stacks a two-word name,
`ctl::shift_legend` puts its LED after the measured text, `bay_effects::ctx::CAPTION_DROP` /
`CAPTION_LINE` keep captions clear of knobs and bezels. Per-control nudges for tight neighbours
live in `panel/layout_tweaks.rs`, never in the `ASSUMED` photo tables (one exception: the
`rotary.speed_leds` rectangle was widened so the row holds its two names). Current look: `docs/img/gui/overview.png` (D246).

| State | Labels >= 7.5 px | Collisions |
|---|---|---|
| Before (original fonts 4.5-7 px, 1560 px panel) | no (smallest 4.5) | 33 |
| 7.5 px minimum at 1560 px | yes | 98 (with the stricter 1 px-gap rule: more) |
| 7.5 px minimum at 3120 px plus 6 nudges (first audit: L and C only) | yes | 0 |
| Same panel, audit extended with K, S, G, N, 0.9 px gap, bezel margin (B8 label legibility) | yes | 30 |
| After the legibility fixes (Rotary box widened, captions, OLED soft labels, SOLO, nudges) | yes | 0 |
| Round 2 rules F, E, T, V, M and bezel 2.5 px added (first run: 9 F, 8 T, 22 V = 39; M and the wider bezel added 1 + 6 on later runs) | yes | 39 |
| After round 2 (drawbar numbers below the cap travel, LEDs before legend text, 9.5 px line pitch, nudges) | yes | 0 |
| Round 3 content added (29 frames, type / timbre names first run: 28; then scale numerals, dial names, drawbar names and numerals: 64 and 43 on the next runs; rules unchanged) | yes | 28 |
| After round 3 (frame pads, group nudges, scale placement, title positions) | yes | 0 |
| Round 5 content added (MODE / VOICE LED names, KB TOUCH names, amp list column, PROGRAM title; nudges in progress) | yes | 1 |
| After round 5 (Piano select nudge 48 / dial 45) | yes | 0 |
| Round 4 content added (ring numerals, variation lists, piano LED names) | yes | 116 first run |
| Round 6 content added (VIB/CHORUS type names, Rotary tab) | yes | 10 first run, 4 then 0 |
| After round 4 (names below numerals, 3 px smaller knobs, frame pads, nudges) | yes | 0 |

## Header tab checks (Organ, Layer Effects)

The raised header tab (graphite since 2026-11-19, `style::HEADER_DARK`) carries the title, FX FOCUS, ON and SOLO on Organ, Piano and Synth, and the title, LAYER FX and its ON button on Layer Effects. Rules kept by tests: the plate outline is painted before the tab so no line runs through the title or the ON button (`widgets/plate.rs`, `tab_is_painted_over_the_body_outline`); the ON LED sits left of its button, inside the tab, never above it (`Face::led_left`, `bay_effects/focus.rs`, `organ_strip.rs` `on_button_led_stays_inside_the_header_tab`); every label and LED printed in the four tabs lies fully inside its tab, measured on the painted text, and Organ, Piano and Synth print an `ON` caption (`panel/header_tab_tests.rs`); the `organ.on` / `organ.solo` nudge in `layout_tweaks.rs` keeps SOLO inside the Organ tab. Look at it with `python tools/gui_shots.py --sections organ,piano,synth,effects` (top 90 px of each PNG). The default baked plate has its own dark headers with accent underlines (D246); the reference render of the whole panel is `docs/img/gui/overview.png` (`nd-matrix-1 --screenshot overview.png --screenshot-scale 1`, trailing empty strip trimmed).

Mod 1 / Mod 2 controls send only the edit they own (`bay_effects/mod_touch.rs`): ON and the type selector the type, Shift + selector the variation or PED, RATE the rate (or PED position / MST CLK division), AMOUNT the amount. A full unit set resent a stale view and, for Mod 2, cleared the firmware raw words (`mod_amount_knob_sends_only_the_amount_and_keeps_a_loaded_mod2`, `every_mod_control_sends_only_its_own_edits`). Known unfixed panel bugs: TD-19 (Organ / Synth SOLO), TD-20 (Mod 2 variation not mirrored after a program load).

## What each switch changes

| Switch (flag / bar control) | Changes | Status |
|---|---|---|
| `--tone-source` (Organ: Source) | `firmware` pipe bank VERIFIED vs SHARC oracle; `b3wheels` ladder VERIFIED 2.5e-11, per-key weights not modelled; `vox`, `farfisa` partial; `legacy` ASSUMED | MIXED per choice |
| `--b3-tail on\|off` (Organ: B3 tail) | B3 processor-tail response (`docs/research/b3-vox-video-gap.md`) | EMPIRICAL |
| `--rotary-drive engine\|unity\|scaled` (Organ: Rotary drive) | `tanh(g x)`, `tanh(g x)/g`, or drive/10 (`docs/research/b3-rotary.md`) | EMPIRICAL |
| `--b3-bass-processor on\|off` (Organ: B3 Bass processor) | B3 Bass key stage (generator weights, window words, line gain of `0x122296`); replaces the B3 tail | MIXED |
| `--farfisa-processor on\|off` (Organ: Farfisa processor) | Key-down weights and line gain of the Farfisa processor `0x122B85` (Farfisa wheels source; partial) | ASSUMED |
| `--leslie firmware\|engine` (Organ: Leslie) | Firmware Leslie processor instead of the engine rotary (`docs/research/fx-leslie.md`) | MIXED (routine VERIFIED, ring EMPIRICAL) |
| `--leslie-ring firmware\|advance\|frozen` (Organ: Ring) | History ring pointers of the firmware Leslie: advance (default), booted pass routine, first oracle state | EMPIRICAL |
| `--leslie-gain-db <dB>` (Organ: Leslie gain) | Gain after the firmware Leslie, -24..+24 dB in the bar | ASSUMED |
| `--organ-level default\|legacy\|firmware\|chain` (Organ: Level) | Organ output level law: legacy bus scale, firmware swell word and squared layer level, or `chain` (verified word 1.6 x FX path 0.3732 in place of K, 14 dB under the B3 / Vox clips) (`docs/research/organ-output-law.md`) | MIXED per choice |
| `--synth-chain legacy\|strip` (Synth: Chain) | Per-voice chain: firmware strip (default since D22, `docs/research/synth-validation.md`) or the classic engine chain; `--legacy-all` selects the classic chain | MIXED (partly VERIFIED) |
| `--synth-knob-laws` / `--no-synth-knob-laws` (Synth: Knob envelope laws) | Strip envelope rate indices straight from the stored knobs (`index = knob - 64`); on by default since D57, off in the Classic preset | VERIFIED (run, [synth-knob-laws](research/synth-knob-laws.md)) |
| `--firmware-synth-velocity` / `--legacy-synth-velocity` (Synth: Firmware velocity) | Amp gain `V` and filter-envelope factor `F` of the ORG core 2 pass instead of the linear velocity blend; on by default since D142 (owner approval, 4 of 16 distinct synth clips marginally worse at replay velocity 100), off in the Classic preset | MIXED (laws and file bits VERIFIED, multiply inside the voice ASSUMED)
| `--legacy-arp` (Synth: Firmware arpeggiator, checked = firmware) | Up / Down / Up-Down walk the whole octave pool and a single held key retriggers every step like the ARM pattern player; on by default since D166 (owner approval 2026-11-18), unchecking restores the engine walk (and turns the gate off). 30 A:16 ARM replays, range law fitted to 14 raw values | MIXED |
| `--no-arp-gate` (Synth: Arp gate 2/3 step, default on since D168) | `E` steps and the plain arpeggio end at 2/3 of the step; the ARM never rewrites the pattern flag after boot (53 ARM-played layers of 30 arp programs), no hardware clip (OQ32) | ASSUMED |
| `--firmware-morph` / `--legacy-morph` (Synth: Firmware morph, checked = firmware) | Wheel, A.T. and Control Pedal weigh `v / 128` (full at 127), clamp on the sum, cutoff sender step drop, five more synth targets play; on by default since D166 (owner approval 2026-11-18), unchecking restores `v / 127` | VERIFIED (245 ARM series) |
| `--synth-stages` / `--no-synth-stages` (Synth: Stage oscillators) | Bit-exact firmware oscillator stages on the strip chain (noise, Shape Square, SUPER, MULTI, SHAPE SINE); on by default | MIXED (stages VERIFIED, voice weights ASSUMED) |
| `--synth-p table\|verified\|fitted\|off\|g,w2,r2,shape,cutoff` (Synth: P table, dropdown entries "Per-wave table", "ARM words (verified)", "Smooth Lead fit", "Off") | Per-wave oscillator level, cutoff factor and SHAPE word law fitted to hardware clips (`synth/p_table.rs`); `verified` = the words the ARM writes per wave type, read by the single-oscillator rows and the SUPER / MULTI stages (`synth/p_words.rs`, `p_stage_words.rs`) | MIXED (words VERIFIED, cutoff factors and level constant EMPIRICAL) |
| `--sample-kernel key\|ratio` (Synth: Sample kernel) | `SAMPLES` oscillator 12-tap kernel pick: firmware key rule (default, D46) or the ratio rule | MIXED (run VERIFIED, key field ASSUMED) |
| `--sample-pitch fw\|equal` (Synth: Sample pitch) | `SAMPLES` step from the ARM exponent tables or exact `2^(n/12)` | MIXED (law VERIFIED, root and rate ASSUMED) |
| `--filter-engine zdf\|firmware` (Synth: Filter) | Firmware synth filter (`synth-engine.md`) | VERIFIED vs oracle |
| `--measured-laws` (Synth: Measured laws) | 16-voice Super Saw, envelope depth, decay, pitch, vibrato (`audible-genius-synth-lessons.md`) | EMPIRICAL |
| `--synth-shape <0..1>` (Synth: Shape) | Shape word of the firmware SHAPE oscillator (strip chain); off keeps the saw placeholder | ASSUMED |
| `--firmware-piano-level` / `--legacy-piano-level` (Piano: Level) | Firmware velocity level law for `.npno` (default on); legacy is the touch curve (`piano-engine.md`) | VERIFIED |
| `--piano-slew-fw` (Piano: SMP gain slew) | `.npno` voices use the firmware slew: hold gain 0.21875 x level with a 2 to 5 s decay, 44 ms release, no extra voice (opt-in; the rate table is EMPIRICAL) | MIXED |
| `--firmware-piano-chain` (Piano: FX path scale) | Sample path scaled 0.7464, the verified FX receiver path (engine unit = DAC fraction ASSUMED; Velvet Grand clip +10 to +12 dB over) | MIXED |
| `--firmware-piano-layer-law` (Piano: Layer law) | Piano layer fader follows `(v / 127)^2` | ASSUMED |
| `--firmware-piano-touch` / `--legacy-piano-touch` (Piano: Touch curves) | KB TOUCH plays the firmware velocity curves (ids 10 / 11 / 13, VERIFIED by ARM run); the Touch bits of every factory program equal the firmware recall (D165); on by default since D165 (owner approval), no Heavy or Light hardware clip, sounds without a level model scale the Medium response (ASSUMED); `--legacy-piano-touch` plays Medium | MIXED |
| `--firmware-piano-bus` (Piano: SMP bus scale) | Sample path scaled 0.5 instead of 0.55 (Q31 receiver convention) | ASSUMED |
| `--firmware-piano-chain` (Piano: FX path scale) | Sample path scaled 0.7464, the verified FX receiver path (engine unit = DAC fraction ASSUMED; Velvet Grand clip +10 to +12 dB over) | MIXED |
| `--amp-path engine\|firmware` (FX: Amp path) | Firmware drive stage (engine default) or engine tanh drive and RBJ cabinet; 4 frames latency | VERIFIED (`tests_amp_golden.rs`) |
| `--firmware-fx` (FX: All ports) | Every `FirmwareFx` port at once, see `effects-and-routing.md`; the bar shows them as the eight checkboxes below | MIXED per port |
| `--firmware-fx-ports delay-laws` (FX: Delay laws) | Delay send and dry gains follow `kDlyMix` | MIXED |
| `--firmware-fx-ports delay-time` (FX: Delay time) | Free-running delay time follows `kDlyRateClkDiv` | VERIFIED |
| `--firmware-fx-ports reverb-mix` (FX: Reverb mix) | Reverb send and dry gains follow `kRvbAmt` | MIXED |
| `--firmware-reverb` (FX: Reverb) | Firmware reverb routines on every chain: Spring (types 0, 1) and multi-tap (types 2..11), 4 frames latency | MIXED (see `fx-reverb-leslie.md`) |
| `--firmware-fx-ports compressor` (FX: Compressor) | Firmware gain computer with the `kCmpAmt` words, 4 frames latency | MIXED |
| `--firmware-fx-ports mod1` (FX: Mod 1) | A-PAN and TREM firmware routines, free running only | MIXED |
| `--firmware-fx-ports mod2` (FX: Mod 2) | Firmware CHOR, PHAS, FLANG, ENS and SPIN stages | MIXED |
| `--firmware-fx-ports eq` (FX: Amp EQ) | Firmware tone stack `0x1243F4` instead of the engine's three biquads, 2 frames latency | MIXED |
| `--firmware-routing` (Global: Firmware routing) | Section/layer enables (a mask of 0 reads as layer A, as the loader does) and per-layer KB zones as stored; on by default since D73, `--legacy-routing` turns it off | VERIFIED |
| `--legacy-1khz-tick` (Global: 24 kHz ARM tick, checked = measured) | Vox key timeline and synth gain de-zipper on the measured 24 kHz ARM timer; on by default since D131, unchecking restores the 1 ms tick. Timer rate VERIFIED, real-time delays are lower bounds (ASSUMED) | MIXED |
| `--legacy-fx-rate` (Global: FX at 43.4 kHz, checked = rate matched) | Firmware Amp drive and cabinet at the DSP frame rate 43,402 Hz instead of once per host frame; on by default since D176, unchecking restores the per-host-frame path. Frame rate VERIFIED, resampling kernel ASSUMED, about 0.8 ms extra latency on amp layers | MIXED |
| `--fx-idle-skip` (Global: Skip idle FX racks) | Stops the FX rack of a layer silent for 4 s until it gets input again; saves most of the firmware FX CPU in a scene that uses few layers. Not a firmware path; a woken rack resumes from frozen state, 4 s and -140 dBFS are ASSUMED (D161) | ASSUMED |
| `--firmware-all` / `--legacy-all` | Presets: verified paths on (not measured laws, Vox/Farfisa/B3) / all defaults | - |

Defaults are unchanged: with no flag the engine and the `render_hash` stay as before. The same flags
work for `ns4 render-patch`; both parse them with `FirmwareOptions::from_args`, so a switch added to
that struct needs a field, a parser, an `apply_exact` line (the GUI path), a per-engine preset entry in `firmware_panel/presets.rs`, `apply_to`/`apply_exact`, a `summary` entry, a `SwitchInfo` in
`firmware_panel/info.rs`, a bar control and a row in this table (a unit test checks the table).

## Library drawer loads

The Pianos and Samples tabs load a sound into a fresh Init Program (only the target layer on, effects
and levels default) unless "Merge into current program" is ticked ([D78](DECISIONS.md)). The
Programs tab replaces the whole state. Evidence, with the local factory backup
(`NS4_FACTORY_BACKUP` or `.reference/`):

```bash
cargo test -p nord-gui --test gui_piano_program_no_organ -- --nocapture
```

The test drives the GUI command path (`panel.apply_program` + `apply_audio_command` of
`LoadProgram`, `LoadPianoSound`, `Keyboard`) and measures the organ energy as the RMS difference to
the same render with both organ faders at zero. It also asserts that program Y after program X
renders exactly like a fresh Y, and that the Organ ladders are unlit while the section is off.

## Text size checks (issue #592, D244)

`Ctrl +` / `Ctrl -` / `Ctrl 0` or the bar pills `A-` `NNN%` `A+` set the text size, 100 to 200 % in steps of 25, saved as `text_scale_percent` in `ns4/config.json` (`--screenshot` always draws 100 %). Headless checks, no window and no audio device (`cargo test -p nord-plugin --lib text_scale`, code `crates/nord-plugin/src/editor/text_scale_tests.rs`, `crates/nord-gui/src/text_scale.rs`, `panel/baked_zoom_base_tests.rs`):

| Check | At sizes |
|---|---|
| Every bar label lies inside the window and its clip rectangle; no two overlap; none taller than the bar | 100, 125, 150, 175, 200 % |
| The `DEBUG` pill text is 10 / 16 / 21 pt high (EMPIRICAL, egui 0.31.1 bundled fonts) | 100, 150, 200 % |
| The Studio Settings dialog (all five tabs) fits inside the editor window and its title grows with the size | 100, 150, 200 % |
| The plate is magnified by the factor and starts at its left edge; `size_for_width` keeps the aspect under the taller bar | 100, 150, 200 % |
| `Ctrl +` / `-` / `0` change and save the size, plain `+` does not | 100 to 150 % |

A golden bitmap is not used: the headless context has no GPU, and the painted text rectangles carry the same information (position, size, clip). Look at it for real with `nd-matrix-1`, press `Ctrl +` twice, open `DEBUG`.
