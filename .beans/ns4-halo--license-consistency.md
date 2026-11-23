---
# ns4-halo
title: License consistency
status: completed
type: task
priority: normal
created_at: 2026-11-23T05:40:00Z
updated_at: 2026-11-23T05:40:00Z
---

## Context

No `LICENSE` file was ever committed, although bean `ns4-815y` claims it added the root `LICENSE` and `crates/nord-plugin/LICENSE`. `Cargo.toml` declares `license = "MIT OR Apache-2.0"` for all crates, which contradicts the source-available non-commercial position in `AGENTS.md` and `README.md`. Recorded as D231.

## Higher Goal

The project's licensing is stated once, consistently, and enforced by a test.

## Acceptance Criteria

- [x] Root `LICENSE` holds the unmodified PolyForm Noncommercial 1.0.0 text plus a `Required Notice:` line.
- [x] Workspace `license = "PolyForm-Noncommercial-1.0.0"` and `publish = false`, inherited by every crate; `pyproject.toml` metadata matches.
- [x] `README.md` has a `## License` section above the Disclaimer.
- [x] `tools/tests/test_license_consistency.py` fails on the old state and passes now.
- [x] D231 recorded in `docs/DECISIONS.md`.

## Out of Scope

- The `crates/nord-plugin` license (GPL question, separate PR); no GPL file is added.
- Contributor license terms.

## Summary of Changes

Added `LICENSE`, switched workspace and Python license metadata to PolyForm-Noncommercial-1.0.0, set `publish = false` for all crates, added the README License section, `tools/license_check.py` with regression tests, and decision D231.
