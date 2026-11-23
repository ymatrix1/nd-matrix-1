from __future__ import annotations

import check_fw_derived as cfd

BASE = """# header
# class B
crates/nord-core/src/organ/b3_bass_fw/prog_gen.rs
# class C
docs/research/data/synth-p-words.csv
# class D (reviewed)
docs/research/data/arm-bulk-tables.json
"""


def baseline() -> dict[str, str]:
    return cfd.parse_baseline(BASE)


def tracked_ok() -> list[str]:
    return [
        "README.md",
        "crates/nord-core/src/lib.rs",
        "crates/nord-core/src/organ/b3_bass_fw/prog_gen.rs",
        "docs/research/data/synth-p-words.csv",
        "docs/research/data/arm-bulk-tables.json",
    ]


def test_parse_baseline_classes():
    assert baseline() == {
        "crates/nord-core/src/organ/b3_bass_fw/prog_gen.rs": "B",
        "docs/research/data/synth-p-words.csv": "C",
        "docs/research/data/arm-bulk-tables.json": "D",
    }


def test_passes_when_tracked_matches_baseline():
    assert cfd.check(tracked_ok(), baseline(), {}) == []


def test_new_derived_file_fails():
    for new in (
        "docs/research/data/pno-fw-image.csv",
        "crates/nord-core/src/fx/fw_new_tables.rs",
        "crates/nord-core/src/organ/x_fw/prog_new.rs",
        "docs/research/data/other-tables.csv",
        "crates/nord-core/src/synth/p-words.txt",
    ):
        errs = cfd.check([*tracked_ok(), new], baseline(), {})
        assert len(errs) == 1 and new in errs[0], new


def test_stale_baseline_entry_fails():
    tracked = [p for p in tracked_ok() if "p-words" not in p]
    errs = cfd.check(tracked, baseline(), {})
    assert len(errs) == 1
    assert "stale baseline entry" in errs[0]
    assert "docs/research/data/synth-p-words.csv" in errs[0]
    assert "remove it" in errs[0]


def test_class_a_files_fail():
    names = (
        "helper.exe",
        "sram.bin",
        "sram.asm",
        "x/Nord.dmg",
        "a/b.ns4b",
        "core1_sram.bin",
        "c_code.bin",
        "s/lib.nsmp4",
        "p/piano.npno",
    )
    for name in names:
        errs = cfd.check([*tracked_ok(), name], baseline(), {})
        assert len(errs) == 1 and "class A" in errs[0], name


def test_unrelated_files_are_not_flagged():
    for ok in (
        "crates/nord-core/src/fx/fw_delay.rs",
        "tools/fw_inspect.py",
        "docs/research/p-words-notes.md",
        "crates/x/src/prog_a.rs",
    ):
        assert cfd.derived_reason(ok) is None, ok


def test_large_binary_allowlisted_vs_not():
    allowed = "docs/img/gui/after-organ.png"
    other = "crates/nord-core/blob.bin"
    sizes = {allowed: 200_000, other: 40_000}
    errs = cfd.check([*tracked_ok(), allowed, other], baseline(), sizes)
    assert len(errs) == 1 and other in errs[0] and "large binary" in errs[0]


def test_small_binary_is_fine():
    p = "crates/nord-core/small.bin"
    assert cfd.check([*tracked_ok(), p], baseline(), {p: cfd.BINARY_LIMIT}) == []


def test_repo_passes_with_committed_baseline():
    tracked = cfd.git_tracked(cfd.ROOT)
    base = cfd.parse_baseline(cfd.BASELINE.read_text(encoding="utf-8"))
    assert cfd.check(tracked, base, cfd.file_sizes(cfd.ROOT, tracked)) == []
