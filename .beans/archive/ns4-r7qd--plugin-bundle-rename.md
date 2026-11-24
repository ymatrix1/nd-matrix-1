---
# ns4-r7qd
title: Ship plugin bundles as nd-matrix-1.clap/.vst3 and fix contact metadata
status: completed
type: task
created_at: 2026-11-23T12:00:00Z
updated_at: 2026-11-23T12:00:00Z
parent: ns4-9wq1
---

## Context
PR #615 left trademark open questions. The shipped bundles were named `nord-plugin.clap` / `nord-plugin.vst3`, which uses "nord" as a brand-like word in our own artifact names. The plugin `EMAIL` (`yuri@github.com`) and Cargo `authors` used an address on a domain the project does not own, and the repository URL still pointed at the old repo name.

## Higher Goal
Our own shipped names and metadata carry no vendor brand and no foreign contact data, without breaking saved DAW projects (D16 stable host IDs).

## Acceptance Criteria
- [x] `bundler.toml` names the bundles `nd-matrix-1.clap` / `nd-matrix-1.vst3`; crate name, Rust identifiers, `CLAP_ID`, `VST3_CLASS_ID`, `NAME` and parameter IDs unchanged
- [x] `release_package.py`, `release_verify.py`, `release_installers.py`, RELEASE_README, RELEASING, plugin-editor and HOST_COMPATIBILITY docs use the new names
- [x] Inno, macOS pkg and AppImage installers delete the legacy `nord-plugin.*` bundles; deb relies on dpkg; dmg and archives documented
- [x] Plugin `EMAIL` is empty, Cargo `authors = ["ymatrix1"]`, `URL` and `repository` are `https://github.com/ymatrix1/nd-matrix-1`, pinned by tests
- [x] Tests updated and added; `tools/check_tests.py` stays at 0
- [x] D16 follow-up line added

## Out of Scope
Renaming the `nord-plugin` crate or `nord-plugin-standalone` binary. Keeping the "Nord OS update" labels (nominative, names the vendor firmware file). The section 10 naming sentence waits for PR #615 to merge.

## Summary of Changes
Added `bundler.toml`; `PLUGIN_STEM` is `nd-matrix-1` and `LEGACY_PLUGIN_STEM` is `nord-plugin`. Installers gained legacy removal (Inno `[InstallDelete]`, macOS `preinstall` scripts, AppRun cleanup). Metadata fixed in `Cargo.toml` and `crates/nord-plugin/src/lib.rs`. Docs and tests updated.
