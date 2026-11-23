"""Build nd-matrix-1 once and write one PNG per panel section (default 2x).

    python tools/gui_shots.py [--out DIR] [--scale 2] [--sections organ,piano] [--no-build]
                              [--backup X.ns4b --program A:14]

Each shot is one `nd-matrix-1 --screenshot ... --screenshot-section ...` run: the window opens
off-screen without focus and the process exits 0 after saving. Every child has a hard timeout
and is killed if it overruns (only the child this script started). Stdlib only.
"""

import argparse
import os
import subprocess
import sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
SECTIONS = ["master", "organ", "piano", "program", "synth", "effects", "keybed"]
# The keybed is 1560 px wide: 1x keeps it inside a normal screen.
SCALE_CAP = {"keybed": 1.0, "master": 2.0}
TIMEOUT_S = 30


def exe_path():
    name = "nd-matrix-1.exe" if os.name == "nt" else "nd-matrix-1"
    return os.path.join(ROOT, "target", "release", name)


def shot_command(exe, out_dir, section, scale, backup=None, program=None):
    """Command line of one section shot; `program` is a panel location (`A:14`) of `backup`."""
    scale = min(scale, SCALE_CAP.get(section, scale))
    tag = program.replace(":", "") + "_" if program else ""
    png = os.path.join(out_dir, f"{tag}{section}.png")
    return [
        exe,
        *(["--backup", backup] if backup else []),
        *(["--program", program] if program else []),
        "--screenshot",
        png,
        "--screenshot-section",
        section,
        "--screenshot-scale",
        str(scale),
    ]


def run_shot(cmd, timeout=TIMEOUT_S):
    """Run one child; kill it and return False when it overruns or fails."""
    proc = subprocess.Popen(cmd)
    try:
        return proc.wait(timeout=timeout) == 0
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait()
        print(f"timeout: {' '.join(cmd[:3])}", file=sys.stderr)
        return False


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--out", default=os.path.join(ROOT, "target", "shots"))
    ap.add_argument("--scale", type=float, default=2.0)
    ap.add_argument("--sections", default=",".join(SECTIONS))
    ap.add_argument("--no-build", action="store_true")
    ap.add_argument("--backup", help="a .ns4b backup to load the program from (never committed)")
    ap.add_argument("--program", help="panel location of the program to load, e.g. A:14")
    args = ap.parse_args(argv)
    os.makedirs(args.out, exist_ok=True)
    if not args.no_build:
        subprocess.run(
            ["cargo", "build", "--release", "-p", "nord-gui", "--bin", "nd-matrix-1"],
            cwd=ROOT,
            check=True,
        )
    failed = []
    for section in args.sections.split(","):
        cmd = shot_command(
            exe_path(), args.out, section.strip(), args.scale, args.backup, args.program
        )
        if run_shot(cmd):
            print(cmd[cmd.index("--screenshot") + 1])
        else:
            failed.append(section)
    if failed:
        print("failed: " + ", ".join(failed), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
