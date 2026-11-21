---
# ns4-d9rr
title: Fix release AppImage build and bump Actions to Node 24
status: in-progress
type: bug
created_at: 2026-11-19T04:45:15Z
updated_at: 2026-11-19T04:45:15Z
---

## Context
Release run 44452016144 failed in the Linux job: appimagetool 1.9.0 runs desktop-file-validate, which was missing on the runner and, once installed, rejects Categories value "Synthesizer" (not a registered freedesktop category). The run also warned Node.js 20 is deprecated (setup-python@v5, upload-artifact@v4, download-artifact@v4, action-gh-release@v2).

## Higher Goal
A tag push produces a complete GitHub Release.

## Acceptance Criteria
- [x] desktop-file-utils installed in the existing apt step (no new step/job)
- [x] Categories uses registered values; unit test guards it
- [x] All actions on Node 24 majors
- [x] workflow_dispatch dry run green on the branch (run 44460412986)

## Out of Scope
Cutting a release or tag; publish job cannot run in a dry run.
