#!/usr/bin/env python3
"""Bump the release version in every place docs/RELEASING.md lists.

    python tools/bump_version.py 0.1.6            # edit files, refresh uv.lock and Cargo.lock
    python tools/bump_version.py 0.1.6 --no-lock  # edit files only

Edits `[workspace.package] version` in Cargo.toml and `[project] version` in
pyproject.toml, then runs `uv lock` and `cargo update --workspace` so the lock
files follow. Used by .github/workflows/prepare-release.yml.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
VERSION_RE = re.compile(r"^\d+\.\d+\.\d+(-[0-9A-Za-z.]+)?$")
# (file, section header regex) pairs; the first `version = "..."` after the header is replaced.
TARGETS = (
    ("Cargo.toml", r"\[workspace\.package\]"),
    ("pyproject.toml", r"\[project\]"),
)


def replace_version(text: str, header: str, new: str) -> tuple[str, str]:
    """Return (new text, old version) with the first version line after `header` replaced."""
    start = re.search(rf"^{header}\s*$", text, re.M)
    if not start:
        raise ValueError(f"section {header} not found")
    line = re.compile(r'^(version\s*=\s*")([^"]+)(")', re.M)
    match = line.search(text, start.end())
    nxt = re.search(r"^\[", text[start.end() :], re.M)
    if not match or (nxt and match.start() > start.end() + nxt.start()):
        raise ValueError(f"no version in section {header}")
    out = text[: match.start(2)] + new + text[match.end(2) :]
    return out, match.group(2)


def bump(root: Path, new: str) -> str:
    """Rewrite all version files under `root`; return the old version. Raises on bad input."""
    if not VERSION_RE.match(new):
        raise ValueError(f"{new!r} is not MAJOR.MINOR.PATCH[-prerelease] (no leading v)")
    results = []
    for name, header in TARGETS:
        path = root / name
        text = path.read_bytes().decode("utf-8")
        results.append((path, *replace_version(text, header, new)))
    olds = {old for _, _, old in results}
    if len(olds) != 1:
        raise ValueError(f"files disagree on the current version: {sorted(olds)}")
    old = olds.pop()
    if old == new:
        raise ValueError(f"version is already {new}")
    for path, text, _ in results:
        path.write_bytes(text.encode("utf-8"))  # bytes: keep the file's line endings
    return old


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("version", help="new version without a leading v, e.g. 0.1.6")
    parser.add_argument("--no-lock", action="store_true", help="skip uv lock / cargo update")
    args = parser.parse_args(argv)
    try:
        old = bump(ROOT, args.version)
    except ValueError as err:
        print(f"error: {err}", file=sys.stderr)
        return 1
    print(f"bumped {old} -> {args.version}")
    if not args.no_lock:
        subprocess.run(["uv", "lock"], cwd=ROOT, check=True)
        subprocess.run(["cargo", "update", "--workspace"], cwd=ROOT, check=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
