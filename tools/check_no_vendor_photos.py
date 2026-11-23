#!/usr/bin/env python3
"""Fail if a vendor product photo is tracked or embedded (D16 "Art and branding").

Checks, over `git ls-files`:
- the two removed Nord product photos never come back (by file name);
- no tracked JPEG under `crates/` or `assets/` unless listed in `JPEG_ALLOWLIST`;
- no Rust source in `crates/` passes a `.jpg` / `.jpeg` path to `include_bytes!`.

Reference photos belong in the git-ignored `.reference/`.
"""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

FORBIDDEN_NAMES = {"panel_texture.jpg", "keyboard_full.jpg"}
JPEG_EXTS = (".jpg", ".jpeg")
JPEG_DIRS = ("crates/", "assets/")
# Tracked JPEGs that are our own art, with the reason. Empty on purpose.
JPEG_ALLOWLIST: set[str] = set()
INCLUDE_JPEG = re.compile(r"include_bytes!\s*\(\s*[^)]*\.jpe?g", re.IGNORECASE)


def tracked_files(root: Path = ROOT) -> list[str]:
    out = subprocess.run(
        ["git", "ls-files"], cwd=root, check=True, capture_output=True, text=True
    ).stdout
    return out.splitlines()


def find_violations(files: list[str], root: Path = ROOT) -> list[str]:
    bad: list[str] = []
    for f in files:
        low = f.lower()
        if Path(low).name in FORBIDDEN_NAMES:
            bad.append(f"{f}: vendor product photo must not be tracked")
        elif low.endswith(JPEG_EXTS) and low.startswith(JPEG_DIRS) and f not in JPEG_ALLOWLIST:
            bad.append(f"{f}: tracked JPEG outside the allowlist (photos stay in .reference/)")
        if low.startswith("crates/") and low.endswith(".rs"):
            path = root / f
            if path.exists() and INCLUDE_JPEG.search(
                path.read_text(encoding="utf-8", errors="replace")
            ):
                bad.append(f"{f}: include_bytes! of a JPEG")
    return bad


def main() -> int:
    bad = find_violations(tracked_files())
    for line in bad:
        print(line)
    if bad:
        return 1
    print("no vendor photos tracked or embedded")
    return 0


if __name__ == "__main__":
    sys.exit(main())
