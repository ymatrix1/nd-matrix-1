#!/usr/bin/env python3
"""Every commit in a range carries a DCO `Signed-off-by:` trailer (D232).

Usage: python tools/check_dco.py BASE..HEAD   (or: BASE HEAD)

Merge commits (more than one parent) are exempt. The core logic works on
plain message strings; git is only used to list the range.
"""

from __future__ import annotations

import re
import subprocess
import sys

SIGNOFF_RE = re.compile(r"^Signed-off-by:[ \t]+\S.*<[^<>\s]+@[^<>\s]+>[ \t]*$", re.MULTILINE)


def has_signoff(message: str) -> bool:
    """True when the message has a `Signed-off-by: Name <email>` line."""
    return SIGNOFF_RE.search(message) is not None


def missing_signoff(commits: list[tuple[str, int, str]]) -> list[str]:
    """Return the shas of non-merge commits without a sign-off.

    `commits` holds `(sha, parent_count, message)`.
    """
    return [sha for sha, parents, msg in commits if parents <= 1 and not has_signoff(msg)]


def read_range(rev_range: str) -> list[tuple[str, int, str]]:
    """List `(sha, parent_count, message)` for every commit in `rev_range`."""
    out = subprocess.run(
        ["git", "log", "--format=%H%x1f%P%x1f%B%x1e", rev_range],
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    commits = []
    for rec in out.split("\x1e"):
        if not rec.strip():
            continue
        sha, parents, msg = rec.strip("\n").split("\x1f", 2)
        commits.append((sha.strip(), len(parents.split()), msg))
    return commits


def main(argv: list[str]) -> int:
    if len(argv) == 2:
        rev_range = argv[1]
    elif len(argv) == 3:
        rev_range = f"{argv[1]}..{argv[2]}"
    else:
        print(__doc__)
        return 2
    bad = missing_signoff(read_range(rev_range))
    if bad:
        print(f"{len(bad)} commit(s) in {rev_range} lack a Signed-off-by trailer:")
        for sha in bad:
            print(f"  {sha}")
        print(
            "Fix with `git rebase --signoff <base>` or `git commit --amend -s`. "
            "See CONTRIBUTING.md."
        )
        return 1
    print(f"All commits in {rev_range} are signed off.")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
