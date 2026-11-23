# Research: DAW plugin dependency licensing (VST3, CLAP, LV2, standalone)

- **Checked date:** 2026-11-23
- **Question:** The core is meant to be PolyForm Noncommercial 1.0.0. `crates/nord-plugin` depends on nih-plug with `features = ["vst3", "standalone"]`. Which dependencies in the plugin tree are non-permissive, does the VST3 path force GPLv3 on the distributed binary, which output formats (VST3, CLAP, LV2) can be shipped without GPL, and what are the options? Technical findings only, not legal advice.
- **Sources (web, all fetched 2026-11-23):**
  - nih-plug `Cargo.toml` (`license = "ISC"`, `vst3-sys` git dependency on `robbert-vdh/vst3-sys` branch `fix/drop-box-from-raw`, `clap-sys` from `micahrj/clap-sys`): <https://raw.githubusercontent.com/robbert-vdh/nih-plug/master/Cargo.toml>
  - nih-plug README (ISC framework, VST3 bindings "licensed under the GPLv3 license", exports "VST3 and CLAP", no LV2): <https://github.com/robbert-vdh/nih-plug>
  - `RustAudio/vst3-sys` README ("licensed under the terms of the GNU GPLv3 license", "a derivative work of the original SDK"): <https://github.com/RustAudio/vst3-sys>
  - `coupler-rs/vst3-rs` (MIT or Apache-2.0, bindings generated from the C++ headers): <https://github.com/coupler-rs/vst3-rs>
  - crates.io `vst3` (0.3.0 published 2025-12-07, `MIT OR Apache-2.0`): <https://crates.io/api/v1/crates/vst3>
  - Steinberg `vst3sdk` README ("VST 3 SDK is under MIT license", GPLv3 and proprietary options "no longer available", SDK 3.8.x): <https://github.com/steinbergmedia/vst3sdk>
  - Search result summaries of the Steinberg announcement (3.8.0 released 2025-10-20 under MIT; VSTGUI and mda files are BSD-like; trademark rules still apply): <https://forums.steinberg.net/t/vst-3-8-0-sdk-released/1011988>, <https://steinbergmedia.github.io/vst3_dev_portal/pages/VST+3+Licensing/VST3+License.html> (search snippet only, see "Not verified")
  - nih-plug issue #181 (maintainer plan to move off GPL bindings, 2024-12-07): <https://github.com/robbert-vdh/nih-plug/issues/181>
  - nih-plug issue #274 (tracking: adopt a permissive binding, opened 2026-06-15, open; a commenter reports nih-plug is not actively maintained and points to the `nice-plug` fork): <https://github.com/robbert-vdh/nih-plug/issues/274>
  - nih-plug PR #263 "Finish switching from vst3-sys to vst3" (open, not merged, last upstream activity 2026-05-17): <https://github.com/robbert-vdh/nih-plug/pull/263>
  - nih-plug issue #281 "Generate LV2 plugins" (open, no replies): <https://github.com/robbert-vdh/nih-plug/issues/281>
  - `micahrj/clap-sys` (MIT and Apache-2.0): <https://github.com/micahrj/clap-sys>
  - `free-audio/clap-wrapper` (MIT; targets VST3, AUv2, AUv3, AAX, standalone; no LV2): <https://github.com/free-audio/clap-wrapper>
  - LV2 `COPYING` (ISC wording, copyright Harris, Robillard, Furse et al.): <https://gitlab.com/lv2/lv2/-/raw/main/COPYING>
  - crates.io `lv2` (0.6.0 published 2020-10-23): <https://crates.io/api/v1/crates/lv2>; `RustAudio/rust-lv2` (Apache-2.0 or MIT, "work in progress", newest commit 2021-11-29): <https://github.com/RustAudio/rust-lv2>
  - DPF (ISC, builds LV2, VST3, CLAP, no Rust support mentioned): <https://github.com/DISTRHO/DPF>
  - crates.io `jack` 0.14.0 (MIT): <https://crates.io/api/v1/crates/jack>
- **Sources (local):** `cargo metadata --format-version 1` and `cargo tree -p nord-plugin -e features -i vst3-sys` in this repo at `origin/main` (nih-plug rev `de421011`, `Cargo.lock`). License strings are the `license` fields of each crate's manifest.

## 0. Requirement (owner constraint, 2026-11-23)

The project must not permit commercial use anywhere. Every shipped plugin binary must therefore be distributable as a whole under PolyForm Noncommercial 1.0.0. Test applied to each dependency: its license must allow adding a noncommercial restriction on the combined binary. Permissive licenses (MIT, Apache-2.0, ISC, BSD, Zlib, Unicode-3.0, CC0, 0BSD, Unlicense, BSL-1.0) pass with attribution. Any copyleft (GPL, LGPL-only, MPL) fails. Result for the current tree: `vst3-sys` (GPLv3) is the only failure; dual licenses with a permissive arm (for example `r-efi`) pass by choosing that arm; `epaint_default_fonts` bundles OFL-1.1 and Ubuntu-font-1.0 fonts, which are permissive for bundling but carry their own notice terms (check, not a copyleft on the binary). GPLv3 (option c) is excluded because it grants commercial use. Technical reading, not legal advice.

## 1. Which crate provides the VST3 bindings, and its license

| Item | Finding | Status |
|---|---|---|
| Binding crate used by nih-plug | `vst3-sys 0.1.0` from `https://github.com/robbert-vdh/vst3-sys.git?branch=fix/drop-box-from-raw#b3ff4d77` (a fork of `RustAudio/vst3-sys`), reached through `nih_plug` (feature `vst3`) | VERIFIED (`cargo tree -i vst3-sys`) |
| License of `vst3-sys` | `GPLv3` (manifest `license` field; upstream README says "a derivative work of the original SDK" under GPLv3) | VERIFIED (cargo metadata, upstream README) |
| License of its COM helpers | `vst3-com`, `vst3-com-macros`, `vst3-com-macros-support`: `MIT` (manifest) | VERIFIED (cargo metadata) |
| License of nih-plug itself | `ISC` (manifest and README); README states the VST3 bindings are GPLv3 | VERIFIED |
| Only non-permissive crate in the plugin tree | `vst3-sys` (GPLv3). Everything else is MIT, Apache-2.0, ISC, BSD, Zlib, BSL-1.0, Unicode-3.0, CC0, 0BSD, Unlicense or a dual license that includes a permissive choice (`r-efi` is `MIT OR Apache-2.0 OR LGPL-2.1-or-later`, so the permissive arm applies) | VERIFIED (script over 507 package versions, all targets and dependency kinds) |

### 1.1 Dependency license table (nord-plugin tree, normal plus build deps, 507 package versions, all targets and dependency kinds)

| License string (as in manifest) | Count | Notable members | Copyleft? |
|---|---|---|---|
| `GPLv3` | 1 | **`vst3-sys 0.1.0`** | **Strong copyleft** |
| `MIT OR Apache-2.0` | 231 | `nord-core`, `nord-gui`, `nord-panel`, `nord-plugin` (workspace license) | No |
| `MIT` | 110 | `vst3-com*`, `jack 0.11.4`, `rfd 0.15.4`, `egui-baseview` | No |
| `Apache-2.0 OR MIT` | 39 | `indexmap`, `async-*` | No |
| `MIT/Apache-2.0` | 23 | `clap-sys 0.5.0` | No |
| `Unicode-3.0` | 18 | `icu_*` | No |
| `Apache-2.0` | 17 | `cpal 0.15.3`, `winit`, `glutin`, `oboe` | No |
| `ISC` | 6 | `nih_plug`, `nih_plug_egui` (vendored), `nih_plug_derive`, `nih_log`, `libloading` | No |
| `MIT OR Apache-2.0 OR LGPL-2.1-or-later` | 2 | `r-efi` (permissive arm usable) | No (choose MIT or Apache) |
| `(MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0` | 1 | `epaint_default_fonts` (font licenses, attribution) | No (font terms) |
| Everything else (one-off permissive strings and dual-license combinations of Zlib, BSD-2/3, BSL-1.0, CC0-1.0, 0BSD, Unlicense, BlueOak) | 59 | `bindgen` BSD-3-Clause, `clipboard-win` BSL-1.0 | No |

Reproduce: `cargo metadata --format-version 1 > meta.json`, then walk `resolve.nodes` from the `nord-plugin` package id and group `packages[].license`.

### 1.2 Format and feature rows

| Format or feature | Provider | License | In tree today | GPL-free? |
|---|---|---|---|---|
| VST3 export (`vst3` feature) | `vst3-sys` (robbert-vdh fork) | GPLv3 | Yes | **No** |
| VST3 SDK headers (Steinberg) | `steinbergmedia/vst3sdk` 3.8.x | MIT (VSTGUI, mda: BSD-like) since 3.8.0, 2025-10-20 | No (not linked by `vst3-sys`) | Yes, but irrelevant to `vst3-sys` |
| Permissive VST3 bindings | `vst3` crate 0.3.0 (coupler-rs) | MIT OR Apache-2.0 | No | Yes |
| CLAP export | `nih_plug` plus `clap-sys 0.5.0` (micahrj) | ISC plus MIT/Apache-2.0 | Yes | **Yes** |
| LV2 spec and headers | LV2 project | ISC (COPYING wording) | No | Yes |
| LV2 export from nih-plug | not supported (README lists VST3 and CLAP; issue #281 open, no replies) | n/a | No | n/a |
| LV2 via Rust | `lv2` 0.6.0 / `rust-lv2` | MIT OR Apache-2.0 (crates.io 0.6.0); repo "Apache-2.0, MIT" | No | Yes, but unmaintained (see s4) |
| LV2 via C++ | DPF | ISC | No | Yes, C++ not Rust |
| CLAP to LV2 bridge | none found (`clap-wrapper` targets VST3, AUv2, AUv3, AAX, standalone) | n/a | No | n/a |
| `standalone` feature backend | `cpal 0.15.3` (Apache-2.0), `jack 0.11.4` (MIT), `jack-sys 0.5.1` (MIT OR Apache-2.0), `midir` path not checked | permissive | Yes | **Yes** |
| Windowing and GUI | `baseview` (MIT OR Apache-2.0), `egui-baseview` (MIT), `rfd` (MIT), `egui` 0.31.1 | permissive | Yes | Yes |

## 2. Did the SDK relicense change the Rust bindings?

- Steinberg relicensed the VST 3 SDK to MIT at 3.8.0, 2025-10-20 (`vst3sdk` README: "VST 3 SDK is under MIT license"; "Licensing under GPLv3 and the Steinberg proprietary license is no longer available").
- That does **not** change `vst3-sys`. Its manifest and README still say GPLv3 and "derivative work of the original SDK", and the nih-plug README repeats that the VST3 bindings are GPLv3. The crate's license is set by its authors, not by Steinberg; relicensing by the SDK owner only removes the reason it had to be GPL. Relicensing `vst3-sys` would need its copyright holders (RustAudio/vst3-sys contributors, plus the fork author) to agree, which nobody has announced.
- `vst3-sys` was GPLv3 in the first place because the SDK was GPLv3-or-proprietary (nih-plug issue #181, maintainer comment 2024-12-07).
- A permissive binding exists: crates.io `vst3` 0.3.0, `MIT OR Apache-2.0`, generated from the C++ headers (needs libclang at build time per issue #181), 242k downloads, releases 0.1.0 (2023-08) to 0.3.0 (2025-12-07).
- nih-plug status for it: PR #263 "Finish switching from vst3-sys to vst3" is open and unmerged (11 commits, ready for review 2026-04-09, no merge decision); PR #236 "Move to non-forked vst3-sys crate" does not fix licensing (upstream is also GPLv3 per issue #274); issue #274 (tracking, 2026-06-15) is open. Issue #274 comments say nih-plug is "not being actively maintained" and point to the community fork `nice-plug` (codeberg.org/RustAudio/nice-plug). A fork (`uchu/nih-plug`) merged #263 into its own branch on 2026-07-20. Search snippets say `nice-plug` is ISC and exports VST3 and CLAP; its VST3 binding was **not** verified (codeberg.org is blocked here).
- The port scope per issue #274: about 4k lines in `src/wrapper/vst3/` and about 110 `vst3_sys` references. The issue lists "persisted `VST3_CLASS_ID` is unchanged" as a check.

## 3. CLAP

- nih-plug exports CLAP through `clap-sys` (`micahrj/clap-sys`, "MIT and Apache-2.0", pinned in nih-plug's manifest to rev `25d7f53f`). The local manifest says `MIT/Apache-2.0`. `nih_plug` itself is ISC.
- No GPL crate is reachable from the `clap` export path. `vst3-sys` is only pulled in by the `vst3` feature (`cargo tree -i vst3-sys` shows it under `nih_plug` only). Whether dropping `"vst3"` from the feature list removes `vst3-sys` from the resolved tree was **not built or tested** in this PR (no Cargo changes allowed). Expected from the feature graph, to confirm in the follow-up.
- A CLAP-only build is therefore expected to have no GPL dependency.

## 4. LV2

- **Spec and headers:** ISC wording in the LV2 repository `COPYING` (fetched from GitLab). The `lv2plug.in` site was unreachable, so the license is taken from the file text only.
- **nih-plug:** README says it "can currently export VST3 and CLAP plugins"; issue #281 "Generate LV2 plugins" (2026-10-25) is open with no comments or maintainer response. No branch or PR found.
- **Rust LV2 crates:** `lv2` 0.6.0 (2020-10-23, `MIT OR Apache-2.0` on the version record; earlier versions ISC). The repo `RustAudio/rust-lv2` is Apache-2.0 or MIT, calls itself "a work in progress", and its newest `develop` commit is 2021-11-29. It is a framework for writing LV2 plugins directly, not a wrapper around an existing nih-plug or CLAP plugin. Using it means re-implementing the plugin shell (ports, state, parameters) and the UI.
- **Wrappers:** `clap-wrapper` (MIT) lists VST3, AUv2, AUv3, AAX and standalone and has no LV2 target (issue search for "lv2" returned 0 results). A web search found no CLAP-to-LV2 bridge for plugin export. DPF (ISC) can build LV2 but is C++ and would need a C ABI shim around `nord-core`.
- **Consequence:** LV2 has no ready path from the current nih-plug code. The cheapest honest route is a hand-written LV2 shell in C or Rust (DPF or `lv2` crate) around `nord-core`, with its own parameter and state mapping, or waiting for nih-plug issue #281.

## 5. `standalone` feature

- `cargo tree -p nord-plugin` shows `cpal 0.15.3` (Apache-2.0), `jack 0.11.4` (MIT), `jack-sys 0.5.1` (MIT OR Apache-2.0) and `rfd 0.15.4` (MIT) under the standalone path. No GPL crate. `baseview` (both revs) is `MIT OR Apache-2.0`; `egui-baseview` is `MIT`.
- `jack-sys` binds the JACK C library at runtime on the host. System libraries (JACK, ALSA, X11, GL) are not crates and were not audited here. Not a source of GPL in the shipped binary as far as the manifests show.
- The standalone binary links `nord-core` and `nord-gui` but not `vst3-sys` when the `vst3` feature is off. When the `vst3` feature is on, `vst3-sys` is compiled into the same crate. The workspace `nord-plugin` crate is a `cdylib` plus `lib` and one `bin`; whether a standalone-only build excludes `vst3-sys` was not tested.

## 6. Repo state relevant to the conflict

- `main` has no `LICENSE` file; workspace `Cargo.toml` says `license = "MIT OR Apache-2.0"`; `docs/OPEN-QUESTIONS.md` OQ6 already notes "the nih-plug VST3 wrapper is GPLv3 if VST3 is enabled".
- Archived bean `ns4-815y` (completed) says it added `crates/nord-plugin/LICENSE` (GPL-3.0-only) and a root PolyForm LICENSE, but neither file exists on `origin/main` as of 2026-11-23 (`ls LICENSE*` fails, `crates/nord-plugin` holds only `Cargo.toml`, `param-ids.snapshot`, `src`, `tests`).
- The plugin ships a vendored `vendor/nih_plug_egui` (ISC) and depends on `nord-core`, `nord-gui`, `nord-panel` by path.

## 7. Implications

1. Any distributed binary that links `vst3-sys` carries a GPLv3 dependency. GPLv3 requires the combined work to be offered under GPLv3, which grants commercial use rights over `nord-core` and `nord-gui` inside that binary. That conflicts with PolyForm Noncommercial 1.0.0 on the same code. (Interpretation of the license text; not legal advice.)
2. The SDK relicensing removes the upstream reason for GPL but not the GPL on `vst3-sys` itself. Only swapping the binding (or an owner relicense of `vst3-sys`, which is not ours) removes it.
3. CLAP and the standalone app are expected to be free of GPL dependencies already.
4. A permissive VST3 binding exists (`vst3` 0.3.0), but nih-plug has not merged the port and appears unmaintained upstream; a fork has it merged.
5. LV2 has no current path from nih-plug and needs a new plugin shell. Not a quick fix for the VST3 gap.
6. Host identity: the VST3 class ID and CLAP ID, the display name, and nih-plug `#[id = "..."]` parameter IDs must not change in any option (D16 additive-only host IDs, enforced by `crates/nord-plugin/param-ids.snapshot`).

## 8. Not verified

- `codeberg.org` (nice-plug repo), `www.steinberg.net`, `steinbergmedia.github.io` and `lv2plug.in` and `lib.rs` were not reachable from this session (network policy). The Steinberg announcement date and the MIT statement were verified from the `steinbergmedia/vst3sdk` README and a search summary, not the press release. nice-plug's license and VST3 binding come from search snippets only.
- Whether `nih_plug` without feature `vst3` drops `vst3-sys`/`vst3-com*` from the build graph: not built.
- Whether the `vst3` crate output satisfies hosts for the existing plugin (never built).
- Whether any copyright holder of `vst3-sys` is willing to relicense: not asked.
- Legal effect of linking and of GPLv3 over a PolyForm-licensed core: needs a lawyer; this file states the technical facts only.
- System libraries linked by the standalone (JACK, ALSA, X11, GL) were not license-audited.

## 9. Follow-ups

- Decide D233 (see [DECISIONS](../DECISIONS.md)).
- Build `nord-plugin` with `features = ["standalone"]` only and with CLAP only, run `cargo tree -i vst3-sys` and confirm it is absent (separate PR, code change).
- If option (b): prototype PR #263 or `nice-plug` against the current plugin, keep `param-ids.snapshot` and VST3/CLAP class IDs identical, test in two hosts.
- Ask the owner whether the LICENSE PR should name nord-plugin explicitly, and align `crates/nord-plugin/Cargo.toml` `license`.
- Re-check nih-plug #263, #274, #281 and `nice-plug` when the decision is taken.
