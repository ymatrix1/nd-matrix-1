# Git workflow

- `main` is current state. Feature branches use `feat/`, `fix/`, `docs/`, `research/`, `chore/`, one concern each.
- Conventional Commits (`feat(organ): ...`, `docs(spec): ...`, `ci: ...`), subject ≤ 72 chars.
- Verify all checks locally before committing and pushing.
- Task / bean / PR descriptions use: **Context**, **Higher Goal**, **Acceptance Criteria**, **Out of Scope**, **Summary of Changes**.
- Never commit `.reference/` (large binary archives > 3 GB), `target/`, `.venv/`, or temporary build artifacts.
- GitHub account is `ymatrix1`.
- **DCO sign-off:** every commit carries `Signed-off-by:` (use `git commit -s`; [DCO 1.1](https://developercertificate.org)). CI runs `tools/check_dco.py` on PR commits. Never commit firmware bytes, disassembly, samples or vendor art ([D20](../../docs/DECISIONS.md#d20-derived-values-only-no-firmware-bytes-in-the-repository), [D128](../../docs/DECISIONS.md#d128-raw-firmware-artifacts-are-never-tracked-derived-data-is-classified-a-to-d-b-and-c-move-to-runtime-loading)). See [docs/LEGAL.md](../../docs/LEGAL.md).
