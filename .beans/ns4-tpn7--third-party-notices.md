---
# ns4-tpn7
title: Ship third-party license notices with every release
status: completed
type: task
priority: high
created_at: 2026-11-23T14:00:00Z
updated_at: 2026-11-23T15:00:00Z
parent: ns4-9wq1
---

## Context
Release archives and installers ship our `LICENSE` (PolyForm Noncommercial 1.0.0) but no notices for the 371 third-party crates (414 name and version pairs) in the binaries (MIT, Apache-2.0, ISC, BSD, Zlib, BSL-1.0, Unicode-3.0, plus OFL-1.1 and Ubuntu-font-1.0 fonts in `epaint_default_fonts`). Those licenses require their copyright and license text to accompany binary distributions. `tools/check_licenses.py` only blocks copyleft.

## Higher Goal
Every distributed binary is accompanied by the legally required notices, and they cannot silently fall out of date when a dependency changes (D238).

## Acceptance Criteria
- [x] `about.toml` (accepted license list, three shipped targets) and `about.hbs` produce `THIRD_PARTY_LICENSES.html` with `cargo about generate`; no crate has an undetermined license
- [x] `release.yml` installs `cargo-about@0.8.4` (`--locked`) and generates the file in every build job
- [x] `tools/release_package.py` ships it in every archive; `tools/release_installers.py` in the Inno, deb, AppImage, dmg and pkg layouts, next to `LICENSE`
- [x] `tools/release_verify.py` fails a release that lacks it or ships a stub
- [x] pytest coverage for package, each installer layout and verify
- [x] CI job `third-party-notices` fails on a license outside the accepted list
- [x] The generated file is not committed (git-ignored)
- [x] `docs/RELEASING.md`, `docs/LEGAL.md` section 4, README License section and D238 updated

## Out of Scope
Licenses of the AppImage runtime embedded by `appimagetool` (not determined, LEGAL.md section 8 question 8); a runtime "About / licenses" dialog in the GUI; changing the project license; code signing.

## Summary of Changes
- New `about.toml`, `about.hbs`; `THIRD_PARTY_LICENSES.html` is generated at release time and git-ignored.
- `release.yml`: cached pinned `cargo-about` install and generation step per build job. `ci.yml`: job `third-party-notices`.
- `release_package.py` (`THIRD_PARTY_NOTICES`, `DOC_FILES`, missing-file error names the command), `release_installers.py` (all layouts), `release_verify.py` (`check_notices`), tests in `tools/tests/test_release_*.py`.
- Docs: `RELEASING.md` section and checklist, `LEGAL.md` section 4 and section 8 question 8, README, D238.
