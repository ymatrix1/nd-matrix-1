## Context

<!-- What is the situation and why is this change needed? -->

## Higher Goal

<!-- Which roadmap phase, epic or bean does this serve? -->

## Acceptance Criteria

- [ ]
- [ ]

## Out of Scope

<!-- What this PR deliberately does not do. -->

## Summary of Changes

<!-- Short list of what changed. -->

Fixes #
Refs #

## Checklist

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- [ ] `cargo test --workspace --all-targets --all-features`
- [ ] `uv run ruff check tools/` and `uv run ruff format --check tools/`
- [ ] `uv run pytest`
- [ ] `python tools/check_tests.py`
- [ ] `python tools/check_fw_derived.py`
- [ ] Every commit is signed off (`git commit -s`, DCO)
- [ ] Specs, `docs/DECISIONS.md` and `docs/ROADMAP.md` checkboxes updated, or not needed
- [ ] No firmware bytes, `.ns4b` data, samples or vendor art (D20, D128)
