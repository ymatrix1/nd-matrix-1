---
# ns4-nyqj
title: 'AppImage catalog compatibility: old glibc, bundled xkbcommon, catalog file name'
status: in-progress
type: task
created_at: 2026-11-24T23:30:12Z
updated_at: 2026-11-24T23:30:12Z
parent: ns4-9wq1
---

## Context
The AppImage catalog (appimage.github.io, PR 12432) tested `nd-matrix-1-v0.1.4-linux-x86_64.AppImage` and failed it: (1) it needs glibc 2.39 (built on `ubuntu-24.04`; the test runs on the oldest supported LTS, 22.04); (2) `Library libxkbcommon-x11.so could not be loaded` and the app exits before a window; (3) the name contains "linux" and a "v" prefix (catalog rule: `<name>-<version>-<arch>.AppImage`).

## Higher Goal
The AppImage runs on any current Linux distribution and is listed in the AppImage catalog.

## Acceptance Criteria
- [x] Linux release job builds on `ubuntu-22.04` (glibc 2.35)
- [x] `libxkbcommon`, `libxkbcommon-x11` and `libxcb-xkb` are bundled in `usr/lib` with their Debian copyright files in `usr/share/doc/nd-matrix-1/bundled-libs/`; `AppRun` sets `LD_LIBRARY_PATH`
- [x] AppImage asset is named `nd-matrix-1-<version>-x86_64.AppImage` (no "linux", no "v")
- [x] Unit tests for the bundling, the name and the workflow runner; decision D248
- [ ] Owner: release a version with the fix (0.1.5 already shipped without it), then comment `/retest` on appimage.github.io PR 12432
- [ ] Verified on a bare Ubuntu 22.04 host with Xvfb (not possible on the Windows dev machine: do it in the release run or a Linux box)

## Out of Scope
The `.deb` dependency list, other distros' catalogs, bundling glibc.

## Summary of Changes
See D248 in `docs/DECISIONS.md`: `tools/release_bundle_libs.py` (new), `tools/release_installers.py`, `.github/workflows/release.yml`, `docs/RELEASING.md`.
