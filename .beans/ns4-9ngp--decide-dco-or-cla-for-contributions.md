---
# ns4-9ngp
title: 'Decide DCO or CLA for contributions'
status: completed
type: task
priority: normal
created_at: 2026-11-23T16:00:00Z
updated_at: 2026-11-23T21:25:00Z
parent: ns4-9wq1
---

## Context
D232 requires DCO sign-off. DCO licenses contributions only under PolyForm Noncommercial, so the owner cannot later relicense or sell commercial licenses for other people's code without each contributor's consent. A CLA requires counsel-reviewed text and adds friction, which does not fit a $0 legal budget for a strictly non-commercial project.

## Higher Goal
The contribution terms match the owner's long-term plans for the project without legal spend.

## Acceptance Criteria
- [x] Owner decides: keep DCO (no CLA) under the $0 legal budget
- [x] `ADDITIONAL-PERMISSIONS.md` updated to cover `ymatrix1 and each contributor to ND Matrix 1` so DCO contributions carry the same musician permission
- [x] Recorded in `docs/LEGAL.md` and epic `ns4-9wq1`

## Out of Scope
Relicensing existing contributions; drafting a custom CLA.

## Summary of Changes
Decided to keep DCO (D232) and skip a CLA under the $0 legal budget. Updated `ADDITIONAL-PERMISSIONS.md` to name `ymatrix1 and each contributor to ND Matrix 1` so external DCO contributions align with `CONTRIBUTING.md`.
