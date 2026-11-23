# Research: permissive VST3 binding for nord-plugin (replace GPLv3 `vst3-sys`)

- **Checked date:** 2026-11-23
- **Question:** Which nih-plug revision has a VST3 wrapper on the permissive `vst3` crate (coupler-rs, `MIT OR Apache-2.0`) instead of the GPLv3 `vst3-sys`, is it maintained enough to pin, and does `nord-plugin` build and keep its host-visible IDs on it?
- **Sources (git over https and crates.io, all fetched 2026-11-23):**
  - nih-plug PR #263 "Finish switching from vst3-sys to vst3", head `refs/pull/263/head` = `95034d55de49b3d792289a0faf5df15b889e86d9` (2026-03-31, Dave Mollen), also the branch `finish-vst3-pr` of `https://github.com/davemollen/nih-plug.git`.
  - nih-plug `master` `de421011` (2026-09-03, the previous pin); merge base of the PR and master is `28b149ec` (2025-09-07).
  - `nice-plug` (`https://github.com/AlexCharlton/nice-plug.git`, ISC), HEAD `50fed0e` 2026-05-29; its commit `a3b5d77` (2026-05-22) "Use vst3 crate instead of vst3-sys".
  - crates.io `vst3` 0.3.0 and `com-scrape-types` 0.1.1 manifests and README (local registry copy).
  - Earlier findings: the plugin licensing research on branch `claude/legal-plugin-license-fvkihn` (PR #614, not on `main` yet).
- **Sources (local):** `cargo tree`, `cargo metadata`, `tools/check_licenses.py`, `cargo test -p nord-plugin`, `cargo xtask bundle nord-plugin`.

## 1. Candidates

| Candidate | VST3 binding | Last commit | License | Port cost for this repo | Verdict |
|---|---|---|---|---|---|
| nih-plug `master` `de421011` (previous pin) | `vst3-sys` (GPLv3) | 2026-09-03 | ISC (framework) | none | rejected: copyleft |
| nih-plug PR #263 head `95034d5` (fork `davemollen/nih-plug`, branch `finish-vst3-pr`) | `vst3` 0.3.0 | 2026-03-31 | ISC | Cargo URL plus `rev` only; same egui, same `nih_plug_egui` sources | **chosen** |
| `nice-plug` `50fed0e` | `vst3` | 2026-05-29 | ISC | Crate renames, egui 0.34 (this repo is on 0.31.1), re-port of the vendored HiDPI `nih_plug_egui` patch | not chosen now: large unrelated churn; revisit if upstream nih-plug stays idle |
| `uchu/nih-plug` (reported to have merged #263 on 2026-07-20) | `vst3` | not inspected | not inspected | unknown | not reachable by search in this session; not needed |

The PR head is 5 upstream commits behind `master` (`28f52b9` nightly SIMD API, rustfmt, a changelog link, a CI release workflow). Those touch `src/buffer/*`, `src/lib.rs` and workflows, not stable code paths this plugin uses. `git diff de421011 95034d5 -- src/params src/plugin src/wrapper/state.rs src/wrapper/util.rs` shows no change outside `src/wrapper/vst3/util.rs` (VST3-only helpers, 2 lines kept, 62 removed).

Why the PR head and not a merged fork: the PR is the upstream-reviewed port (11 commits on top of `28b149ec`), the branch is pinned by commit hash, and the diff to the old wrapper is confined to `src/wrapper/vst3/*` and `src/buffer/*`. A fork that merged it into a divergent branch would add unreviewed changes. Maintenance risk: the pin is a PR branch of a personal fork, not a release. Mitigation: `rev` is a full commit hash in `Cargo.lock`; the commit is also reachable as `refs/pull/263/head` on the upstream repository. If the PR merges upstream, switch the URL back to `robbert-vdh/nih-plug` at the merge commit.

## 2. License of every crate the switch adds or removes (VERIFIED, `cargo metadata`)

| Crate | Change | License |
|---|---|---|
| `vst3` 0.3.0 | added | `MIT OR Apache-2.0` (README: "distributed under the terms of both the MIT license and the Apache license, version 2.0"; bindings generated from the Steinberg headers and published pre-generated, so no libclang or SDK needed at build time) |
| `com-scrape-types` 0.1.1 | added | `MIT OR Apache-2.0` |
| `vst3-sys` 0.1.0 | removed | `GPLv3` |
| `vst3-com`, `vst3-com-macros`, `vst3-com-macros-support` | removed | `MIT` |
| `nih_plug`, `nih_plug_derive`, `nih_plug_xtask` (pin changed) | same crates, new rev | `ISC` |

Result: `cargo tree -p nord-plugin -i vst3-sys` reports no match; `python3 tools/check_licenses.py` reports 500 third-party crates, no failure. Run against the old lockfile the same script fails with `vst3-sys 0.1.0: copyleft-only license 'GPLv3'`.

Not verified: the Steinberg SDK headers behind the generated bindings. The SDK has been MIT since 3.8.0 (2025-10-20); which SDK version `vst3` 0.3.0 was generated from is not stated in the crate. Owner to confirm if a legal review is wanted.

## 3. Identity stability (D16)

| Item | Evidence |
|---|---|
| CLAP ID `org.ndmatrix.nd-matrix-1` | `plugin_ids::tests::clap_id_is_pinned` |
| VST3 class ID `NDMatrix1Emulato` (16 bytes) | `plugin_ids::tests::vst3_class_id_is_pinned` (also the Windows byte order of the first 8 bytes) |
| Host parameter ID strings, step counts, defaults | `crates/nord-plugin/param-ids.snapshot` unchanged (`git diff --stat` shows no change), `param_snapshot::tests::shipped_ids_are_stable` passes |
| VST3 numeric parameter IDs (hash of the string ID) | nih-plug `src/wrapper/util.rs` (`hash_param_id`) byte-identical at `de421011` and `95034d5`; `plugin_ids::tests::hash_matches_pinned_values` pins four IDs and `snapshot_ids_hash_without_collision` checks all 815 are distinct |
| Plugin state blob | `src/wrapper/state.rs` and `src/params/` unchanged between the revs |

## 4. Implications

- No shipped binary in the plugin tree carries a copyleft license; CI job `licenses` keeps it that way.
- VST3 hosts see the same class ID and parameter IDs, so existing projects should load. This is not host-tested (no DAW here): see the checklist in `docs/HOST_COMPATIBILITY.md` section 7.10.
- D233 records option (b) as accepted (owner, 2026-11-23); OQ45 is answered there.

## 5. Follow-ups

- Run the host checklist (HOST_COMPATIBILITY 7.10) in at least one VST3 host and the Steinberg `validator`.
- When PR #263 merges upstream, repin to the upstream merge commit.
- Re-evaluate `nice-plug` if upstream nih-plug stays unmaintained and egui needs to move past 0.31.1.
