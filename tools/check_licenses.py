#!/usr/bin/env python3
"""No copyleft crate in the shipped `nord-plugin` dependency tree (D233).

Usage: python tools/check_licenses.py [--package nord-plugin]

The project license is PolyForm Noncommercial 1.0.0, so no shipped binary may link a crate
whose license permits commercial use only under copyleft terms (GPL, AGPL, LGPL, MPL, SSPL,
EUPL, and similar). An SPDX `A OR B` expression passes when at least one alternative is
free of copyleft terms (for example `MIT OR LGPL-2.1-or-later`, r-efi). `AND` requires every
term to pass. Dev-dependencies are not shipped and are ignored. Workspace crates are skipped.

The core logic works on plain strings and a `cargo metadata` dict; cargo is only run by `main`.
"""

from __future__ import annotations

import json
import re
import subprocess
import sys

COPYLEFT = re.compile(
    r"^(A?GPL|LGPL|MPL|SSPL|EUPL|CDDL|EPL|OSL|CPL|CC-BY-SA|CC-BY-NC|Sleepycat|RPL|Watcom)",
    re.IGNORECASE,
)
TOKEN = re.compile(r"\(|\)|[^\s()]+")


def is_copyleft_id(license_id: str) -> bool:
    """True for a single SPDX identifier (with optional `+` or `WITH` exception stripped)."""
    return COPYLEFT.match(license_id.rstrip("+")) is not None


def _parse_or(tokens: list[str], pos: int) -> tuple[bool, int]:
    ok, pos = _parse_and(tokens, pos)
    while pos < len(tokens) and tokens[pos].upper() == "OR":
        rhs, pos = _parse_and(tokens, pos + 1)
        ok = ok or rhs
    return ok, pos


def _parse_and(tokens: list[str], pos: int) -> tuple[bool, int]:
    ok, pos = _parse_atom(tokens, pos)
    while pos < len(tokens) and tokens[pos].upper() == "AND":
        rhs, pos = _parse_atom(tokens, pos + 1)
        ok = ok and rhs
    return ok, pos


def _parse_atom(tokens: list[str], pos: int) -> tuple[bool, int]:
    if pos >= len(tokens):
        raise ValueError("unexpected end of license expression")
    tok = tokens[pos]
    if tok == "(":
        ok, pos = _parse_or(tokens, pos + 1)
        if pos >= len(tokens) or tokens[pos] != ")":
            raise ValueError("unbalanced parenthesis in license expression")
        return ok, pos + 1
    if tok.upper() in {"AND", "OR", "WITH", ")"}:
        raise ValueError(f"unexpected token {tok!r} in license expression")
    pos += 1
    if pos < len(tokens) and tokens[pos].upper() == "WITH":  # `X WITH exception`
        pos += 2
    return not is_copyleft_id(tok), pos


def expression_is_permissive(expr: str) -> bool:
    """True when the SPDX expression can be satisfied without any copyleft license."""
    tokens = TOKEN.findall(expr.replace("/", " OR "))  # legacy `MIT/Apache-2.0` form
    ok, pos = _parse_or(tokens, 0)
    if pos != len(tokens):
        raise ValueError(f"trailing tokens in license expression {expr!r}")
    return ok


def shipped_packages(meta: dict, root_name: str) -> list[dict]:
    """Registry/git packages reachable from `root_name` through normal or build dependencies."""
    by_id = {p["id"]: p for p in meta["packages"]}
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    roots = [p["id"] for p in meta["packages"] if p["name"] == root_name]
    if not roots:
        raise ValueError(f"package {root_name!r} not in cargo metadata")
    seen: set[str] = set()
    stack = list(roots)
    while stack:
        pid = stack.pop()
        if pid in seen:
            continue
        seen.add(pid)
        for dep in nodes[pid]["deps"]:
            if any(k.get("kind") != "dev" for k in dep["dep_kinds"]):
                stack.append(dep["pkg"])
    workspace = set(meta["workspace_members"])
    return sorted(
        (by_id[p] for p in seen if p not in workspace and by_id[p].get("source")),
        key=lambda p: (p["name"], p["version"]),
    )


def violations(packages: list[dict]) -> list[str]:
    """One line per package whose license is missing, unparsable or copyleft-only."""
    bad: list[str] = []
    for pkg in packages:
        expr = pkg.get("license")
        label = f"{pkg['name']} {pkg['version']}"
        if not expr:
            bad.append(f"{label}: no SPDX license expression")
            continue
        try:
            ok = expression_is_permissive(expr)
        except ValueError as err:
            bad.append(f"{label}: {err}")
            continue
        if not ok:
            bad.append(f"{label}: copyleft-only license {expr!r}")
    return bad


def main(argv: list[str]) -> int:
    root = "nord-plugin"
    if "--package" in argv:
        root = argv[argv.index("--package") + 1]
    out = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--locked"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    packages = shipped_packages(json.loads(out), root)
    bad = violations(packages)
    print(f"{root}: {len(packages)} third-party crates checked")
    for line in bad:
        print(f"FAIL {line}")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
