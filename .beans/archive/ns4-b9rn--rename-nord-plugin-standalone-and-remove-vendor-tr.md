---
# ns4-b9rn
title: Rename nord-plugin-standalone and remove vendor prefix from shipped binaries
status: completed
type: task
priority: high
created_at: 2026-11-23T21:25:00Z
updated_at: 2026-11-24T13:50:47Z
parent: ns4-9wq1
---


## Context
PR #619 renamed the DAW plugin bundles to `nd-matrix-1.clap` and `nd-matrix-1.vst3`, but `tools/release_package.py` (`BINS = ("ns4", "nd-matrix-1", "nord-plugin-standalone")`) and `README.md` still ship a user-facing executable named `nord-plugin-standalone[.exe]`.

## Higher Goal
No shipped user-facing binary, installer entry, or archive carries the vendor's `nord-` trademark prefix.

## Acceptance Criteria
- [x] Rename the standalone plugin binary to `nd-matrix-1-standalone` (or drop it from release archives if redundant with `nd-matrix-1`)
- [x] Update `tools/release_package.py`, `tools/release_installers.py`, `tools/release_verify.py`, `README.md`, and tests
- [x] Add a `release_verify.py` check that fails if any shipped binary or bundle starts with `nord-`

## Out of Scope
Internal Rust crate directory names (`crates/nord-*`, `publish = false`).

## Summary of Changes
- Kept and renamed the binary: `nd-matrix-1-standalone` is the nih-plug standalone host of the plugin editor (plugin engine, host-style parameters, own audio device selection), not a duplicate of the `nd-matrix-1` egui GUI, so dropping it would remove a shipped feature.
- `crates/nord-plugin/Cargo.toml` `[[bin]] name` and `.github/workflows/release.yml` build step; crate directory names unchanged.
- `tools/release_package.py`: `STANDALONE_BIN`, `BINS`, `LEGACY_STANDALONE_BIN`. `tools/release_installers.py`: DEB/AppDir/DMG/PKG staging, AppRun `--standalone-plugin`, Inno Setup source and `[InstallDelete]` of the old `.exe`.
- `tools/release_verify.py`: `check_vendor_names` fails any archive, folder, file or bundle name starting with `nord-`; tests in `tools/tests/test_release_verify.py` fail without it.
- README, RELEASE_README.txt, RELEASING.md (also fixed stale `nord-stage-4-emulator-*` archive names), HOST_COMPATIBILITY.md, ROADMAP, LEGAL, epic bean.
