---
# ns4-9wq1
title: 'Legal protection program'
status: in-progress
type: epic
priority: normal
created_at: 2026-11-23T16:00:00Z
updated_at: 2026-11-23T21:25:00Z
---

## Context
On 2026-11-23 the project closed its main legal gaps: a real PolyForm Noncommercial license (D231), DCO and docs/LEGAL.md (D232), the firmware-derived ratchet (D128), trademark cleanup in UI and bundles, removal of vendor photos, a permissive VST3 binding with a copyleft gate (D233), third-party notices in releases (D238), musician permission and trademark policy (D241), and a history rewrite purging vendor files.

Updated 2026-11-23 for a **$0 legal budget**: prioritize concrete, zero-cost engineering mitigations (removing remaining Class B/C firmware-derived files, neutral non-red default panel theme, removing `nord-` from shipped binaries, removing in-GUI monetization, keeping DCO without a CLA) and defer paid counsel / trademark registration unless the project ever commercializes or receives a legal notice.

## Higher Goal
ND Matrix 1 stays a strictly non-commercial, source-available interoperability project with a strong $0-budget legal posture: zero vendor material in tree or history, clean licensing, distinct visual identity, and a prompt takedown response process.

## Acceptance Criteria
Done:
- [x] ns4-halo License consistency (D231)
- [x] ns4-lgl8 Legal posture and DCO (D232)
- [x] ns4-r4tq Firmware-derived ratchet (D128)
- [x] ns4-tm7q Trademark UI strings
- [x] ns4-3o8r Remove vendor photos
- [x] ns4-r7qd Plugin bundle rename and contact metadata
- [x] ns4-q7lp Plugin licensing research (D233)
- [x] ns4-q7vn Updater terms and patent question
- [x] ns4-tpn7 Third-party notices (D238)
- [x] ns4-9ngp Keep DCO (no CLA) under the $0 legal budget; `ADDITIONAL-PERMISSIONS.md` updated to cover contributors
- [x] ns4-b9rn Standalone binary renamed to `nd-matrix-1-standalone`; `release_verify.py` rejects `nord-` names
- [x] ns4-sgro Licenses of the AppImage runtime shipped in Linux releases (D242, PR #638)
- [ ] ns4-nyqj AppImage catalog compatibility: glibc 2.35 build, bundled xkbcommon, catalog file name (D248; needs a release and `/retest`)
- [x] History rewrite of 2026-11-23 (docs/LEGAL.md section 8 question 5; `v0.1.0`–`v0.1.2` tags moved)

Open — Tier 1 ($0 quick wins):
- [x] ns4-b9rn Rename `nord-plugin-standalone` and remove vendor prefix from shipped binaries
- [x] ns4-s3pn Remove in-GUI sponsorship button to keep strictly non-commercial posture (README badge and FUNDING.yml: owner decision open)
- [ ] ns4-vt4r Verify GitHub Releases for moved tags and request GitHub Support purge of old PR refs (Owner action)
- [ ] ns4-6fbu Repository settings: required checks and description (Owner action)
- [x] ns4-d7mg Check macOS `.dmg` updater and `nordkeyboards.com` terms (free check)

Open — Tier 2 ($0 high-impact engineering):
- [ ] ns4-u8o2 Move firmware-derived files to runtime loading (**Class B code images & `native_bass_*.rs` first**, then Class C tables)
- [ ] ns4-eh57 Second history rewrite after the firmware migrations
- [ ] ns4-80xe Proactive trade dress mitigation: neutral non-red default panel theme
- [ ] ns4-pvb3 Permissive VST3 binding: DAW host checks

Deferred / Optional (only if commercializing or contacted by a rights holder):
- [ ] ns4-4qmh Additional permission for musicians (shipped in D241; counsel review deferred)
- [ ] ns4-0s8y Trademark policy for ND Matrix 1 (shipped in D241; paid registration deferred)
- [ ] ns4-ns2j Counsel review of docs/LEGAL.md open questions (deferred under $0 budget)

## Out of Scope
Product features unrelated to legal risk.

## Summary of Changes
In progress.
