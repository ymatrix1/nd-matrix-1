<h1 align="center">ND Matrix 1</h1>

<p align="center">
  <strong>Source-available (non-commercial) DSP sound engine, binary preset toolkit (<code>CBIN</code> / <code>.ns4b</code> / <code>.ns4p</code>), and High-Level Emulator for the Nord Stage 4.</strong><br>
  <sub>Built for interoperability, research, preservation, and compatibility purposes.</sub>
</p>

---

The **Nord Stage 4** (`product_id=46`) is built around a **three-engine, 7-layer multitimbral DSP architecture** running across an **`ARM0`** host controller and parallel Analog Devices SHARC floating-point DSPs (**`DSPO`** Organ, **`DSPS`** Synth, **`DSPF`** Effects & Piano Resonance).

Unlike generic ROMpler clones that only imitate the front panel visually, this project implements the real internal signal architecture and binary preset formats documented through empirical hardware and firmware analysis:

- **Clavia Binary (`CBIN`) & Backup (`.ns4b`) Interoperability:** Lossless parsing, inspection, and round-trip serialization of `.ns4p` (868 B Programs), `.ns4l` (868 B Live slots), `.ns4y` (541 B Synth presets), `.ns4n` (195 B Piano presets), `.ns4o` (183 B Organ presets), `.ns4t` (124 B Settings), `.ns4b` backup archives, and OS v1.66 firmware containers.
- **Organ Engine (`DSPO` — 2 Layers: A/B):** 100% mathematical physical modeling of 91 phase-locked spinning B3 tonewheels with authentic 196-tooth gear ratios, harmonic foldback, key click, single-triggered 2nd/3rd harmonic percussion, C1–C3/V1–V3 scanner vibrato, Vox/Farfisa/Pipe models, and the **Model 122 Vintage Rotary Speaker** (800 Hz Linkwitz-Riley crossover, independent horn/drum rotational ballistics, and stereo Doppler delay modulation).
- **Piano Engine (`DSPF` — 2 Layers: A/B, 120-Voice Polyphony):** Multisample playback paired with real-time **Advanced String Resonance** (12-band tuned modal sympathetic resonator bank excited when the damper pedal is depressed), **Dynamic Pedal Noise**, velocity response curves (`Light`, `Medium`, `Heavy`), and Timbre EQ presets (`Soft`, `Mid`, `Bright`, `Dyno1`, `Dyno2`).
- **Synth Engine (`DSPS` — 3 Layers: A/B/C, Nord Wave 2 Derived):** Anti-aliased **PolyBLEP** Virtual Analog oscillators (`Saw`, `Pulse/PWM`, `Triangle`, `Sine`), **Wavetable**, **2/3-Operator FM**, and **SFZ/WAV Sample** modes feeding topology-preserving **Zero-Delay Feedback (`ZDF`)** filters (`12/24 dB Classic Transistor Ladder`, `LP`, `BP`, `HP`), ADR envelopes, LFOs, and a clock-synced Arpeggiator.
- **7 Independent Per-Layer Effects Chains:** Every one of the 7 layers (`Organ A/B`, `Piano A/B`, `Synth A/B/C`) runs its own dedicated 5-stage stereo FX pipeline (`Mod 1` → `Mod 2` → `Amp/Drive/3-Band EQ` → `Ping-Pong/Analog Delay` → `Reverb`), summed into the global **Master Compressor**, **Master Clock**, and `CS4392` + `LM833` post-DAC reconstruction filter.
- **4-Zone Keybed Router, Layer Scene II & 3-Source Morphing Matrix:** Up to 3 split points (4 zones) with `Off`, `Small` (6 st), and `Large` (12 st) crossfades, instant **Layer Scene I/II** switching, and simultaneous **3-Source Morphing** (`Mod Wheel` CC1, `Control Pedal` CC11, `Aftertouch`).

---

## Architecture & Documentation

Read the full specification suite in [`docs/`](docs/README.md):

| Document | Contents |
|---|---|
| [docs/PRD.md](docs/PRD.md) | Goals, target users, latency & polyphony budgets, non-goals |
| [docs/ROADMAP.md](docs/ROADMAP.md) | Phased milestones and parallel development lanes |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | System diagram, crate layout, and real-time audio invariants |
| [docs/DECISIONS.md](docs/DECISIONS.md) | Architectural Decision Records (`D1`–`D10`) |
| [docs/specs/cbin-and-presets.md](docs/specs/cbin-and-presets.md) | 48-byte `CBIN` header, `.ns4p`/`.ns4y`/`.ns4o`/`.ns4n` bitstream embedding, OS v1.66 container |
| [docs/specs/organ-engine.md](docs/specs/organ-engine.md) | 91-tonewheel B3 math, foldback, percussion, and Model 122 Rotary Speaker |
| [docs/specs/synth-engine.md](docs/specs/synth-engine.md) | PolyBLEP VA oscillators and Zero-Delay Feedback (`ZDF`) 4-pole Ladder filter |
| [docs/specs/piano-engine.md](docs/specs/piano-engine.md) | Modal sympathetic string resonance, pedal noise, and timbre filters |
| [docs/specs/effects-and-routing.md](docs/specs/effects-and-routing.md) | 7 per-layer FX racks, Master Compressor, and Master Clock |
| [docs/specs/morph-and-zones.md](docs/specs/morph-and-zones.md) | 4-zone split crossfades, Layer Scene II, and 3-source Morphing Matrix |
| [docs/specs/midi-and-control.md](docs/specs/midi-and-control.md) | Complete Nord Stage 4 MIDI CC table |
| [docs/research/](docs/research/) | Empirical teardown notes of `.ns4b`, OS v1.66 firmware, and DAC schematics |

---

## Download

**Playing the GUI:** `nd-matrix-1` plays from a MIDI keyboard (first MIDI port, velocity passed through unchanged), the mouse (velocity 88) or the computer keyboard (`A S D F G H J K L ; '` white keys, `W E T Y U O P` black keys, `Z` / `X` octave, `C` / `V` velocity, `Space` sustain; see [GUI testing](docs/gui-testing.md)).

Tagged releases ship `ns4` (CLI), `nd-matrix-1` (GUI), `nord-plugin-standalone`, and the CLAP + VST3 DAW plugins in multiple formats (all with `.sha256` checksums):
- **Windows (x86_64):** `-setup.exe` installer (with selectable GUI, CLI, VST3 & CLAP components) and portable `.zip`
- **macOS (arm64):** `.pkg` installer (selectable app, VST3 & CLAP), `.dmg` disk image, and portable `.tar.gz`
- **Linux (x86_64):** `.deb` package, portable `.AppImage` (run with `--install-plugins` to copy CLAP/VST3 to `~/.clap` and `~/.vst3`), and `.tar.gz`

All builds are unsigned and provided **AS IS, without warranty or support of any kind**. The plugin has **not** been verified in any DAW yet ([docs/HOST_COMPATIBILITY.md](docs/HOST_COMPATIBILITY.md)). Maintainers: see [docs/RELEASING.md](docs/RELEASING.md).

**Real WAVE sounds (optional, one time):** the WAVE wavetables are Nord firmware and are not in this repository. Download the official Nord OS update and give that file to the app (`nd-matrix-1`: button "Nord OS update file", or `ns4 firmware set <file>`); see [Where to get the Nord OS update](docs/guides/nord-os-update.md). Without it a placeholder wavetable plays. Only `nd-matrix-1` and `ns4` use the saved file; the plugin (CLAP/VST3) and `nord-tools` do not read it; they keep the placeholder wavetable.

---

## Quick Start

### 1. Build & Test the Rust DSP & Parser Core (`crates/nord-core`)

```bash
cargo test --workspace --all-targets --all-features
```

Run the `ns4` CLI to inspect a preset/backup or render an offline 7-layer audio test buffer:

```bash
cargo run -p nord-core --bin ns4 -- info
cargo run -p nord-core --bin ns4 -- render-demo --frames 48000
```

### 2. Inspect `.ns4b` Backups, `.ns4p` Programs & Firmware Images (`tools/ns4_inspect.py`)

```bash
uv sync --locked
uv run python tools/ns4_inspect.py --help
```

### 3. Run Full CI Quality Gates Locally

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
uv run ruff check tools/
uv run ruff format --check tools/
uv run pytest
python tools/check_tests.py
```

---

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for the full contribution guide. Before opening a pull request:
- Read [AGENTS.md](AGENTS.md) and [.agents/rules/](.agents/rules/) for project conventions.
- Tasks are tracked with [beans](https://github.com/hmans/beans) in [`.beans/`](.beans/).
- Every source file in `crates/` and `tools/` must have unit tests (`python tools/check_tests.py` enforces 0 uncovered files in CI).

---

## License

This project is source-available under the [PolyForm Noncommercial License 1.0.0](LICENSE). You may use, study, modify, and share it for any noncommercial purpose (personal use, research, hobby projects, and noncommercial organizations); commercial use needs separate permission from the licensor. The crates are not published to crates.io (`publish = false`).

The licensing of the DAW plugin (`crates/nord-plugin`) is under review; until that is settled it is covered by the same license.

---

## Disclaimer

This software is provided **"as is"**, without warranty or support of any kind.

The developers of this repo are not affiliated, associated, endorsed by, or in any way officially connected with Nord Keyboards / Clavia DMI AB, or any of its subsidiaries or its affiliates. The official Nord Keyboards website can be found at https://www.nordkeyboards.com. The names Nord and Clavia as well as related names, marks, emblems and images are registered trademarks of their respective owners.

You must supply your own lawfully obtained firmware (OS update) and backup files. This repository ships no vendor installers, firmware images, samples, factory presets or vendor art. Some firmware-derived tables are still tracked and are being moved to runtime loading from your own OS update file (D128, see docs/LEGAL.md). Files written by this software and loaded onto hardware are used at your own risk: back up your keyboard first, because a malformed file can lose data or leave the instrument in an unexpected state. The legal posture and the rights-holder contact process are in [docs/LEGAL.md](docs/LEGAL.md); it is a technical record, not legal advice.
