---
# ns4-r4tq
title: Ratchet firmware-derived files so they can only shrink (accept D128)
status: completed
type: task
created_at: 2026-11-23T06:00:00Z
updated_at: 2026-11-23T06:00:00Z
parent: ns4-trkb
---

## Context

D128 was `proposed`; the repo owner approved it on 2026-11-23. Class B (code images) and class C (numeric table dumps) files are still tracked, several embedded with `include_str!`. The migrations to runtime loading need the owner's OS update file for verification and cannot be done in the agent environment (tracked in `ns4-u8o2`). Until then nothing stops new derived files from being added.

## Higher Goal

The repository holds only our own work; firmware-derived data can only leave, never enter.

## Acceptance Criteria

- [x] D128 is `accepted 2026-11-23` with `Decided by` set and the ratchet named in its Impact.
- [x] `tools/check_fw_derived.py` fails on stale baseline entries, new derived-looking files, class A files and unlisted binaries over 32 KB; logic is pure functions over a file list.
- [x] `tools/fw-derived-baseline.txt` lists every tracked class B (7) and class C (31) path plus 2 reviewed class D false positives, and the check passes on the current tree.
- [x] `tools/tests/test_check_fw_derived.py` covers pass, new derived file, stale entry, class A file, large binary allowlisted vs not.
- [x] CI step runs the check; AGENTS.md "Checks before pushing", the provenance rule and the inventory ("Enforcement") mention it.
- [x] Inventory gaps fixed (four class C files were not named).

## Out of Scope

The actual B/C migrations to runtime loading (`ns4-u8o2`). History rewrite (owner decision).

## Summary of Changes

Added `tools/check_fw_derived.py`, `tools/fw-derived-baseline.txt`, `tools/tests/test_check_fw_derived.py`, a CI step, doc updates (D128, inventory, AGENTS.md, provenance rule) and this bean. `ns4-u8o2` now lists each B/C file as an unchecked criterion.
