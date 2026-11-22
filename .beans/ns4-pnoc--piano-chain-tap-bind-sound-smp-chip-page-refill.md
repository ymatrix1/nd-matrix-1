---
# ns4-pnoc
title: "Piano chain tap: bind a piano sound, SMP chip in sharc-live, page refill"
status: todo
type: task
priority: normal
created_at: 2026-11-19T12:00:00Z
updated_at: 2026-11-19T12:00:00Z
parent: ns4-trkb
---

## Context
155 piano programs have no chain tap (`piano_no_chain_tap`). [piano-chain-tap](docs/research/piano-chain-tap.md) (OQ41, D170) shows a piano key sends no SMP message in the census harness and lists four blockers.

## Higher Goal
Audio 1:1 with hardware from ARM + SHARC execution only: compare native and chain for piano programs.

## Acceptance Criteria
- [x] Live driver binds a piano sound (`arm_piano_load` stub); program to `.npno` map documented (D185, `arm_piano_bind.py`)
- [x] `smp` preset and model in `sharc-live` (template `live_sy.rs`) (D185, `live_smp.rs`)
- [x] CNSP page refill of the SMP ring from the bound `.npno` (D191, `smp-page-refill.md`: law verified, rings equal the file)
- [x] Voice advances after the refill and the tap is the piano (D199 harness fix, D205 pacing: history equals the reference decode)
- [x] Piano programs in the census and ranking with a level, flag `piano_no_chain_tap` removed (D214: 152 of 155 programs; 3 give no audio)

## Out of Scope
Default flips, FX chip live (OQ36).

## Summary of Changes (2026-11-21, partial)
Steps 1 and 2 done, see [piano-chain-tap-smp](../docs/research/piano-chain-tap-smp.md) and D185. Remaining: page refill, census and ranking column. Bean stays `todo`.

2026-11-21 (D191): page refill done, voice phase freezes after the first page; next blocker in [smp-page-refill](../docs/research/smp-page-refill.md) s4.

2026-11-21 (D205): the D199 tap was a runaway decode; with `--fifo-pace 1` it is a piano (12 of 12 pilot programs). Mechanism for the ranking column done (`program_rank_chain.py --smp`); the 155-program run and the interrupt rate are open, see [smp-voice-pitch](../docs/research/smp-voice-pitch.md).

2026-11-21 (D214): the 155-program census ran: 152 piano programs have a chain tap in `program-ranking.csv`, 3 none (zone gain 0 at key 60 twice, piano layer B once). Tool fixes: tap log dir, layer A fallback in the bind, `program_rank_smp.py`.
