---
# ns4-d7mg
title: Check macOS .dmg updater and Nord website terms (free check)
status: completed
type: task
priority: normal
created_at: 2026-11-23T21:25:00Z
updated_at: 2026-11-24T03:35:06Z
parent: ns4-9wq1
---


## Context
`docs/LEGAL.md` section 2 checked the Windows v1.66 `.exe` updater with `strings` and found no EULA or reverse-engineering restriction, but left the macOS `.dmg` updater and `nordkeyboards.com` download page / terms of use unchecked.

## Higher Goal
Close the remaining factual gaps in `docs/LEGAL.md` section 2 without paid counsel.

## Acceptance Criteria
- [x] Run a text/resource inspection on a local, git-ignored `Update Nord Stage 4 OS v1.66.dmg` (DMG license resource + strings inside the `.app` bundle)
- [x] Inspect `nordkeyboards.com` Downloads page and site footer/terms for any license or reverse-engineering terms
- [x] Replace the remaining direct `.zip` deep-link constants in `crates/nord-core/src/firmware_setting.rs` and `crates/nord-gui/src/debug_dialog.rs` with the Downloads page URL (matching `docs/guides/nord-os-update.md`)
- [x] Record factual findings and date in `docs/LEGAL.md` section 2 and section 8 question 1

## Out of Scope
Legal conclusions or paid counsel review (`ns4-ns2j`).

## Summary of Changes
- macOS `.dmg` (v1.66) inspected 2026-11-23 with 7-Zip into a scratch dir (nothing executed): no licence resource in the DMG plist, no licence or terms file in the volume, no EULA or reverse-engineering strings in `Nord OS Update`, `Info.plist`, `CodeResources` or `os.cab`.
- nordkeyboards.com read 2026-11-24 (no login, no cookie or terms accepted, no download): the Downloads pages show no terms; the footer has only "Copyright © 2026 Clavia DMI AB. All rights reserved." and no terms-of-use link; guessed terms URLs return 404.
- `firmware_setting::DOWNLOAD_WINDOWS` / `DOWNLOAD_MAC` replaced by `DOWNLOADS_PAGE` (Stage 4 Downloads page); GUI `OS_UPDATE_URL`, the OS update window and `ns4 firmware status` text now point to the page; tests added.
- `docs/LEGAL.md` sections 2 and 8 Q1 and `docs/guides/nord-os-update.md` updated (facts only, not legal advice).
