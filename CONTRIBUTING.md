# Contributing to ND Matrix 1

Thank you for your interest in contributing to **ND Matrix 1** (`nord-stage-4-emulator`). This project is built for **interoperability, research, preservation, and compatibility** with the Clavia Nord Stage 4 (`product_id=46`).

---

## Ways to Contribute

- **DSP & Engine Accuracy:** Improve the Organ (`DSPO`), Piano (`DSPF`), Synth (`DSPS`), or Effects pipelines, or help close gaps in [docs/OPEN-QUESTIONS.md](docs/OPEN-QUESTIONS.md) and [docs/TECH_DEBT.md](docs/TECH_DEBT.md).
- **Binary & Firmware Interoperability:** Expand lossless `CBIN` (`.ns4p`, `.ns4l`, `.ns4y`, `.ns4n`, `.ns4o`, `.ns4t`), `.ns4b`, and firmware container support.
- **GUI, CLI & DAW Plugin:** Improve `nd-matrix-1`, `ns4`, or CLAP/VST3 host compatibility ([docs/HOST_COMPATIBILITY.md](docs/HOST_COMPATIBILITY.md)).
- **Tests & Documentation:** Add deterministic unit tests, verify hardware/firmware behaviour, or keep [docs/specs/](docs/specs/README.md) up to date.

---

## Getting Started

### Prerequisites

- **Rust** toolchain (`cargo`, `rustfmt`, `clippy`)
- **Python** managed with [`uv`](https://docs.astral.sh/uv/)

### Local Setup

```bash
# Clone the repository
git clone https://github.com/your-org/nord-stage-4-emulator.git
cd nord-stage-4-emulator

# Sync Python dependencies for tools/ and tests
uv sync --locked

# Build and run the Rust test suite
cargo test --workspace --all-targets --all-features
```

> **Note:** Large local reference files (such as `.ns4b` backups, OS updater binaries, and PDFs) belong in `.reference/`, which is git-ignored. Never commit proprietary firmware or copyrighted sample libraries. Unit tests must use deterministic synthetic fixtures in memory.

---

## Finding & Tracking Work

- **Roadmap:** See [docs/ROADMAP.md](docs/ROADMAP.md) for current phases and parallel lanes.
- **Tasks (`beans`):** Tasks are tracked in [`.beans/`](.beans/) using [`beans`](https://github.com/hmans/beans) (prefix `ns4-`). Run `beans list --ready` to see ready tasks, and commit `.beans/` updates alongside the changes they track.

---

## Project Conventions & Rules

Full engineering rules live in [AGENTS.md](AGENTS.md) and [`.agents/rules/`](.agents/rules/). The key invariants are:

1. **Docs & specs first:** Any change to behaviour, binary layout, DSP signal flow, or public API must update its specification in [`docs/specs/`](docs/specs/README.md) and [`docs/ROADMAP.md`](docs/ROADMAP.md) in the same PR. Architectural decisions go in [docs/DECISIONS.md](docs/DECISIONS.md).
2. **Real-time DSP safety:** Audio synthesis code (`organ.rs`, `piano.rs`, `synth.rs`, `fx.rs`, `engine.rs`) must never allocate on the heap, lock a mutex, or perform I/O inside `process_block()` or `render_frame()`.
3. **Lossless `CBIN` round-tripping:** Parsing and re-serializing `.ns4p`, `.ns4y`, `.ns4o`, `.ns4n`, `.ns4l`, or `.ns4t` files must preserve the 48-byte `CBIN` header and raw bitstream payload so unedited fields remain bit-identical.
4. **Provenance in code:** Tag hardware/firmware constants as `VERIFIED`, `EMPIRICAL`, or `ASSUMED` with their source and verification check.
5. **Every source file ships unit tests:** Every `.rs` and `.py` source file must have unit tests. `python tools/check_tests.py` enforces 0 uncovered files in CI.

---

## Quality Gates

Run all checks locally before opening a pull request (these mirror `.github/workflows/ci.yml`):

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
uv run ruff check tools/
uv run ruff format --check tools/
uv run pytest
python tools/check_tests.py
```

---

## Pull Request Guidelines

- **Commits:** Use [Conventional Commits](https://www.conventionalcommits.org/) (e.g., `feat:`, `fix:`, `docs:`, `test:`, `chore:`).
- **PR Description:** Include **Context**, **Higher Goal**, **Acceptance Criteria** (`- [ ]` / `- [x]`), **Out of Scope**, and **Summary of Changes**.
- **Evidence:** Include the output or confirmation of the passing quality gates above.
