---
# ns4-c6kq
title: Follow OS updates automatically
status: todo
type: task
created_at: 2026-11-27T12:00:00Z
updated_at: 2026-11-27T12:00:00Z
parent: ns4-lle1
---

## Context
OS v1.68 (2026-11-26) was a recompile of v1.66 ([os-1-68-vs-1-66](docs/research/os-1-68-vs-1-66.md), D257): the DSP images were identical, the ARM side moved. Layout scans (WAVE registry, fonts, DSP image chain) already follow updates; the Rust ARM0 harness (`nord-arm` `Layout`) and the Bass `A` line offsets need one derived table per version.

## Higher Goal
A new Nord OS update works with no code change: the app and tools read the user's own update file and find every address they need.

## Acceptance Criteria
- [ ] `Layout` addresses of `nord-arm` found at run time (anchors: strings, call graph, `tools/fw_addr_map.py` logic ported to Rust), `V166` / `V168` kept as tests of the finder
- [ ] Bass `A` line offsets found by content (the delta names the words, the finder locates them) instead of `dm_text::OS_V168`
- [ ] A check that runs the finder against every update in `.reference/` (v1.44 to the newest) and reports boot / WAVE / font / Bass status per version
- [ ] Release-time note: a new update triggers `tools/fw_addr_map.py` and the check above

## Out of Scope
Porting the Unicorn scripts `tools/arm_*.py`; SHARC changes (images identical in v1.68).

## Summary of Changes
(open)
