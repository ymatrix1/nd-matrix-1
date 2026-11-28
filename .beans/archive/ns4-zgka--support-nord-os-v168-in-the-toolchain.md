---
# ns4-zgka
title: Support Nord OS v1.68 in the toolchain
status: completed
type: task
priority: normal
created_at: 2026-11-28T03:50:30Z
updated_at: 2026-11-28T03:50:52Z
parent: ns4-lle1
---

## Context
OS v1.68 is a recompile of v1.66 (docs/research/os-1-68-vs-1-66.md, D257).

## Higher Goal
Every tool reads the user's own v1.68 update file.

## Acceptance Criteria
- [x] Analysis note and fw_addr_map.py mapper
- [x] nord-arm Layout per OS version boots v1.68
- [x] WAVE registry, UI fonts, bar font found by layout scan
- [x] Bass A line shifts keyed per OS version
- [x] ns4 inspect prints the OS version

## Out of Scope
Runtime discovery of Layout addresses (ns4-c6kq); v1.68 .ns4p/.ns4b (OQ44).

## Summary of Changes
See PR.
