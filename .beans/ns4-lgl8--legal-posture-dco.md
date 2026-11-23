---
# ns4-lgl8
title: 'Legal posture doc, DCO sign-off, disclaimer and rights-holder contact (D232)'
status: completed
type: task
priority: normal
created_at: 2026-11-23T12:00:00Z
updated_at: 2026-11-23T12:00:00Z
---

## Context
The project reverse engineers the Nord Stage 4 firmware and file formats for interoperability. Nothing recorded the legal posture, required contributor sign-off, or gave rights holders a contact path.

## Higher Goal
Make the project's technical legal posture explicit and enforceable (not legal advice), and keep contributions clean of firmware bytes and vendor art.

## Acceptance Criteria
- [x] `docs/LEGAL.md` records purpose, reference sourcing, no circumvention, what ships, trademarks, third-party references, takedown contact and open questions
- [x] README Disclaimer expanded and linked to LEGAL.md
- [x] CONTRIBUTING.md "Licensing of contributions" (DCO 1.1) and the git-workflow rule updated
- [x] `tools/check_dco.py` with unit tests and a PR-only CI job `dco`
- [x] `.github/ISSUE_TEMPLATE/legal.md` for rights-holder contact
- [x] D232 recorded in `docs/DECISIONS.md`

## Out of Scope
The LICENSE file, Cargo.toml license fields and the README License section (separate change). Migration of class B and C tables (D128 follow-ups). Rewriting git history. Legal review by counsel.

## Summary of Changes
Added `docs/LEGAL.md`, D232, the DCO checker, its CI job and tests, the legal issue template; expanded the README Disclaimer, CONTRIBUTING.md and the git workflow rule. Existing history is not checked by the DCO job.
