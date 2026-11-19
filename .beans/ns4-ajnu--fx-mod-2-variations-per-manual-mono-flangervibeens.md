---
# ns4-ajnu
title: 'FX: Mod 2 variations per manual (mono Flanger/Vibe/Ensemble, Chorus variation, Ensemble 3-line, Spin)'
status: todo
type: task
created_at: 2026-11-12T03:26:34Z
updated_at: 2026-11-12T03:26:34Z
parent: ns4-trkb
---

## Context
Manual p.50: variation of FLANGER/VIBE/ENS = mono; CHORUS variation = vintage pedal; ENS = three cross-connected modulated delay lines; SPIN = neutral rotary motion, variation shifts treble to lower frequencies. Today Ensemble renders as Chorus and Spin as Vibe (ASSUMED), variation bit unused.

## Acceptance Criteria
- [ ] Mod2Type::Ensemble and Spin with variation flag
- [ ] Mono vs stereo versions distinguishable in tests (L/R coherence)
- [ ] Tagged constants

## Progress 2026-11-18
The firmware ports for these FX exist (opt-in `FirmwareFx`). The ranked-distance evidence for flipping them (D163, `docs/research/program-ranking-fx.md`): `FirmwareFx::PARITY` takes the 384-program mean score from 5.35 to 1.66; default flip proposed, owner decision pending.
