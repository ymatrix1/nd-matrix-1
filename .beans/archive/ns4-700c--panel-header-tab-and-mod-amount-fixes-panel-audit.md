---
# ns4-700c
title: Panel header tab and Mod amount fixes, panel audit
status: completed
type: task
priority: normal
created_at: 2026-11-19T03:00:01Z
updated_at: 2026-11-19T03:19:58Z
---

## Context
Owner screenshot of the ORGAN header (ON/SOLO tab) plus a known Mod 2 AMOUNT bug.

## Higher Goal
GUI panel matches the reference and sends only the edit that changed.

## Acceptance Criteria
- [x] Header tab: no line through title/button, ON LED inside tab, SOLO inside tab (Organ, Layer Effects)
- [x] Mod 1/2 AMOUNT sends only the amount
- [x] Unfixed bugs in TECH_DEBT (TD-19, TD-20)

## Out of Scope
Audio crackle, nord-gui audio runtime, SOLO engine wiring.

## Summary of Changes
See PR.
