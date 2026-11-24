---
# ns4-s3pn
title: 'Remove in-GUI sponsorship button to keep strictly non-commercial posture'
status: completed
type: task
priority: high
created_at: 2026-11-23T21:25:00Z
updated_at: 2026-11-23T21:25:00Z
parent: ns4-9wq1
---

## Context
PR #635 added a GitHub Sponsors button inside the GUI, a README badge, and `.github/FUNDING.yml`. Accepting donations inside an emulator of a currently-sold commercial instrument weakens the non-commercial research/interoperability posture and introduces ambiguity under PolyForm Noncommercial 1.0.0 when combined with DCO third-party contributions.

## Higher Goal
The shipped application has zero monetization UI so the project remains unmistakably non-commercial under a $0 legal budget.

## Acceptance Criteria
- [x] Remove the GitHub Sponsors button from the GUI (`crates/nord-gui`)
- [x] Owner decided: remove the README Sponsors badge and `.github/FUNDING.yml` (done)
- [x] All quality gates pass

## Out of Scope
Changing the project license.

## Owner decision (open)

Not touched in this change; both are owner-only and the README/GitHub UI side of the Sponsors feature:

- README badge: `README.md` line 9 (`img.shields.io/badge/Sponsor-GitHub`, links to `github.com/sponsors/ymatrix1`).
- `.github/FUNDING.yml` (`github: [ymatrix1]`): shows the Sponsor button on the GitHub repo page.

Recommendation: delete both for the same non-commercial reason, and disable Sponsors on the GitHub account. Tracked as a follow-up once decided.

## Summary of Changes

- Deleted `crates/nord-gui/src/sponsor.rs` (URL, label, one test), its `pub mod sponsor;` in `lib.rs`, and the `SPONSOR` title-bar pill in `main.rs`.
- Added `crates/nord-gui/tests/no_monetization.rs`: fails if any `nord-gui` source names a sponsor, donate, PayPal, Ko-fi or Patreon control.
- `docs/specs/visual-style-and-panel.md` section 5 item 4 records the no-monetization rule; `docs/ROADMAP.md` box ticked.
- Plugin crate never had the pill (grep of `crates/`, `docs/`, `tools/`).
