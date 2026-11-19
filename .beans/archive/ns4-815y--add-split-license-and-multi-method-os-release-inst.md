---
# ns4-815y
title: Add split license and multi-method OS release installers
status: completed
type: task
priority: normal
created_at: 2026-11-17T01:44:27Z
updated_at: 2026-11-17T01:57:47Z
---

## Context
The project currently only releases raw .zip/.tar.gz archives on GitHub Releases and lacks root LICENSE files.

## Higher Goal
Support multiple distribution methods (native OS installers for Windows, macOS, and Linux, including optional/selectable CLAP and VST3 DAW plugin installation, alongside portable archives) and establish clear licensing (PolyForm-Noncommercial-1.0.0 for emulator/core/GUI/CLI/tools, GPL-3.0-only for nord-plugin).

## Acceptance Criteria
- [x] Add root LICENSE (PolyForm-Noncommercial-1.0.0) and crates/nord-plugin/LICENSE (GPL-3.0-only) and update Cargo.toml license fields
- [x] Add native OS installer packaging (Windows Inno Setup .exe, macOS .dmg + .pkg with .app bundle, Linux .deb + .AppImage) with standalone + selectable CLAP/VST3 plugin installation
- [x] Update .github/workflows/release.yml to build, checksum, and publish all release artifacts
- [x] Add unit tests in tools/tests/ for all new/updated release tooling and verify release_verify.py
- [x] Update docs/RELEASING.md, docs/RELEASE_README.txt, and README.md
- [x] All CI quality gates pass (cargo fmt, clippy, test, ruff, pytest, check_tests.py)

## Out of Scope
- Code signing / Apple notarization / Authenticode (builds remain unsigned)

## Summary of Changes
- Added `LICENSE` (PolyForm-Noncommercial-1.0.0), `crates/nord-plugin/LICENSE` & `LICENSE-PLUGIN` (GPL-3.0-only), and updated `Cargo.toml` and `crates/nord-plugin/Cargo.toml`.
- Created `tools/release_installers.py` and `tools/tests/test_release_installers.py` for Windows Inno Setup (`-setup.exe`), macOS (`.dmg`, `.pkg` with `Nord Stage 4.app`), and Linux (`.deb`, `.AppImage`), all packaging standalone apps + CLAP/VST3 plugins.
- Updated `.github/workflows/release.yml`, `docs/RELEASING.md`, `docs/RELEASE_README.txt`, and `README.md` with the new formats, split license, and AS-IS/no-support disclaimer.
