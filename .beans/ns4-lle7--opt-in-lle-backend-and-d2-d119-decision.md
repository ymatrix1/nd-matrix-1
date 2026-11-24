---
# ns4-lle7
title: 'Opt-in LLE backend in nd-matrix-1/nord-plugin and D2/D119 decision'
status: in-progress
type: task
priority: normal
created_at: 2026-11-23T21:40:00Z
updated_at: 2026-11-23T21:40:00Z
parent: ns4-lle1
blocked_by:
    - ns4-lle6
---

## Context
D2 currently designates the high-level Rust DSP engine as the primary engine and D119 keeps the SHARC system run as a proposed opt-in. Once `ns4-lle6` proves a playable real-time LLE backend, the project can expose it in `nd-matrix-1` and `nord-plugin` and decide whether LLE replaces the firmware-derived HLE paths — which would also allow deleting all Class B/C firmware-derived files (`ns4-u8o2`) in one stroke while keeping only clean-room `CBIN`/`.ns4p`/`.ns4b` tools and pure math fallback DSP.

## Higher Goal
Users can select the real-time LLE backend when they supply their own OS update file, and the architectural decision (opt-in vs. default replacement of firmware-derived HLE) is recorded in `docs/DECISIONS.md`.

## Acceptance Criteria
- [x] Expose an opt-in `--engine lle` / Settings toggle in `nd-matrix-1` and `nord-plugin` when the user has configured their local OS update file (selection seam only: the backend is a stub reporting `Unavailable`)
- [x] Graceful fallback (or clear prompt to supply the OS update file) when no OS update file is configured
- [ ] Verify real-time stability in standalone and in DAW hosts (CLAP/VST3) (open: no LLE engine exists yet)
- [ ] Record the architectural decision updating D2/D119 in `docs/DECISIONS.md` (D246 written as `proposed`; the final decision waits for Gate 3 evidence and the user) and coordinate with `ns4-u8o2` / `ns4-eh57` on retiring firmware-derived HLE tables
- [x] User-selectable SHARC float precision `Exact` / `Fast` (`--precision`, `NS4_PRECISION`, saved key, Settings radio enabled only with LLE; plumbed to the stub backend seam; Fast tolerance still to be measured by `ns4-lle3`)
- [x] Unit tests for engine selection and missing-firmware fallback

## Out of Scope
Shipping any firmware or sample files; paid counsel review.

## Notes (selection half, 2026-11-23)
Done in this branch: `engine_select` (EngineKind, stub LleBackend, resolve and fallback), `engine` config key, `--engine` in `ns4` and `nd-matrix-1`, Engine toggle in the shared OS & Wave Tables panel (standalone and plugin), D246 proposed, specs and roadmap. Open: the real backend (blocked by lle5/lle6), DAW-host stability, the final D2/D119 decision, and `ns4-u8o2` / `ns4-eh57` coordination (gated on Gate 3).
