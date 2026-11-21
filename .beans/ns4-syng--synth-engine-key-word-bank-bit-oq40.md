---
# ns4-syng
title: Synth Engine: why the ARM sends one voice bank bit (OQ40)
status: todo
type: task
priority: low
created_at: 2026-11-19T12:00:00Z
updated_at: 2026-11-19T12:00:00Z
parent: ns4-trkb
---

## Context
Synth Engine's arp step key word is `0x0100E200` (bank bit 0x01 only); the DSP voice is silent, with both bits it sounds ([silent-programs](docs/research/silent-programs.md) s6, OQ40).

## Higher Goal
Know whether hardware sounds this step; close the last silent chain program.

## Acceptance Criteria
- [ ] Trace key word builder `0xC01EC690` args for one arp step
- [ ] Read voice records of both banks in the live link
- [ ] Decide native vs firmware for the step

## Out of Scope
Native changes before the hardware behaviour is known.
