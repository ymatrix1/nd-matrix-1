#!/usr/bin/env python3
"""Ratchet: firmware-derived files can only shrink (D128).

D128 (docs/DECISIONS.md) says raw firmware artifacts are never tracked and that
class B (code images) and class C (numeric table dumps) move to runtime loading.
Until each migration lands, `tools/fw-derived-baseline.txt` lists every tracked
class B and C path. This check fails when:

- a baseline path is no longer tracked (stale entry: remove it, the ratchet only
  goes down);
- a tracked file looks firmware-derived and is not in the baseline (new B or C
  file, or a class A raw artifact);
- a tracked binary file larger than `BINARY_LIMIT` is not in `BINARY_ALLOWLIST`.

Baseline format: one path per line; `# class B`, `# class C` and `# class D`
section comments set the class of the paths that follow. Other `#` lines are
comments. Class D lines are heuristic false positives reviewed by hand: they
are exempt from detection but still must stay tracked (stale check applies).

Detection heuristics (path based, deliberately simple):

- class A: `*.exe`, `*.dmg`, `*.ns4b`, `*.nsmp4`, `*.npno`, `*_sram.bin`,
  `*_code.bin` (mirrors .gitignore) and the root names `sram.bin`, `sram.asm`,
  `helper.exe`.
- `*-fw-image.*` anywhere.
- `fw_<name>_tables.rs` anywhere under `crates/`.
- `prog_*.rs` under a `*_fw/` directory.
- under `docs/research/data/` or `crates/`: file names containing `fw-image`,
  `-tables.` or `p-words`.
"""

from __future__ import annotations

import fnmatch
import subprocess
import sys
from pathlib import Path, PurePosixPath

ROOT = Path(__file__).resolve().parents[1]
BASELINE = ROOT / "tools" / "fw-derived-baseline.txt"

CLASS_A_PATTERNS = (
    "*.exe",
    "*.dmg",
    "*.ns4b",
    "*.nsmp4",
    "*.npno",
    "*_sram.bin",
    "*_code.bin",
)
CLASS_A_ROOT_NAMES = ("sram.bin", "sram.asm", "helper.exe")

# A tracked binary above this size must be in BINARY_ALLOWLIST.
BINARY_LIMIT = 32 * 1024
BINARY_SUFFIXES = (".png", ".jpg", ".jpeg", ".gif", ".bin", ".dat", ".wav", ".ogg", ".zip", ".gz")
# Own work, not firmware: GUI panel art rendered by us (assets/blender, nord-panel),
# panel proof renders, and GUI screenshots. Reviewed 2026-11-23 against the inventory
# (class D, "Not firmware"). Provenance of the source photos is inventory follow-up 6.
BINARY_ALLOWLIST = (
    "assets/panel/proof/*.png",
    "crates/nord-gui/assets/baked/*.png",
    "docs/img/gui/*.png",
)


def parse_baseline(text: str) -> dict[str, str]:
    """Return {path: class letter}. Lines before any class header get class '?'."""
    out: dict[str, str] = {}
    cls = "?"
    for raw in text.splitlines():
        line = raw.strip()
        if not line:
            continue
        if line.startswith("#"):
            words = line.lstrip("#").split()
            if len(words) >= 2 and words[0].lower() == "class" and words[1][:1] in "ABCD":
                cls = words[1][:1]
            continue
        out[line] = cls
    return out


def class_a_reason(path: str) -> str | None:
    name = PurePosixPath(path).name
    if path in CLASS_A_ROOT_NAMES:
        return "class A root artifact"
    for pat in CLASS_A_PATTERNS:
        if fnmatch.fnmatch(name, pat):
            return f"class A pattern {pat}"
    return None


def derived_reason(path: str) -> str | None:
    """Why `path` looks firmware-derived, or None."""
    a = class_a_reason(path)
    if a:
        return a
    p = PurePosixPath(path)
    name = p.name
    if fnmatch.fnmatch(name, "*-fw-image.*"):
        return "name matches *-fw-image.*"
    in_crates = p.parts[:1] == ("crates",)
    if in_crates and fnmatch.fnmatch(name, "fw_*_tables.rs"):
        return "name matches fw_*_tables.rs"
    if (
        in_crates
        and fnmatch.fnmatch(name, "prog_*.rs")
        and any(part.endswith("_fw") for part in p.parts[:-1])
    ):
        return "prog_*.rs under *_fw/"
    if in_crates or path.startswith("docs/research/data/"):
        for needle in ("fw-image", "-tables.", "p-words"):
            if needle in name:
                return f"name contains {needle}"
    return None


def is_large_binary_candidate(path: str, sizes: dict[str, int]) -> bool:
    return path.lower().endswith(BINARY_SUFFIXES) and sizes.get(path, 0) > BINARY_LIMIT


def binary_allowed(path: str) -> bool:
    return any(fnmatch.fnmatch(path, pat) for pat in BINARY_ALLOWLIST)


def check(tracked: list[str], baseline: dict[str, str], sizes: dict[str, int]) -> list[str]:
    """Pure check. Returns human readable error strings (empty list = pass)."""
    errors: list[str] = []
    tracked_set = set(tracked)
    for path in sorted(baseline):
        if path not in tracked_set:
            errors.append(
                f"stale baseline entry: {path} is no longer tracked; "
                "remove it from tools/fw-derived-baseline.txt (the ratchet only goes down)"
            )
    for path in sorted(tracked_set):
        if path in baseline:
            continue
        reason = derived_reason(path)
        if reason:
            errors.append(
                f"new firmware-derived file: {path} ({reason}); do not track it "
                "(D128). Load it at runtime from the user's OS update file instead"
            )
        elif is_large_binary_candidate(path, sizes) and not binary_allowed(path):
            errors.append(
                f"large binary: {path} ({sizes[path]} B > {BINARY_LIMIT} B) is not in "
                "BINARY_ALLOWLIST in tools/check_fw_derived.py; do not track it unless it "
                "is our own work, then add a reviewed allowlist entry"
            )
    return errors


def git_tracked(root: Path) -> list[str]:
    out = subprocess.run(
        ["git", "ls-files", "-z"], cwd=root, check=True, capture_output=True
    ).stdout.decode("utf-8")
    return [p for p in out.split("\0") if p]


def file_sizes(root: Path, paths: list[str]) -> dict[str, int]:
    sizes: dict[str, int] = {}
    for p in paths:
        f = root / p
        if f.is_file():
            sizes[p] = f.stat().st_size
    return sizes


def main() -> int:
    tracked = git_tracked(ROOT)
    baseline = parse_baseline(BASELINE.read_text(encoding="utf-8"))
    errors = check(tracked, baseline, file_sizes(ROOT, tracked))
    if errors:
        print("Firmware-derived ratchet failed (D128):")
        for e in errors:
            print(f"  - {e}")
        return 1
    counts: dict[str, int] = {}
    for c in baseline.values():
        counts[c] = counts.get(c, 0) + 1
    summary = ", ".join(f"class {k}: {v}" for k, v in sorted(counts.items()))
    print(f"Firmware-derived ratchet OK ({len(baseline)} baseline paths; {summary}).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
