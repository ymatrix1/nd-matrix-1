#!/usr/bin/env python3
"""Count the x86 instructions of the `uop_probe_*` functions of the SHARC uop engine (D260).

Usage: python tools/uop_insn_count.py --build [--check]
       python tools/uop_insn_count.py --asm FILE.s [--check]
       python tools/uop_insn_count.py OBJECT [...] [--check]
       objdump -d -r -C --no-show-raw-insn X.o | python tools/uop_insn_count.py --text - [--check]

Rust has no in-process instruction counter, so the budgets of the kernels are measured on the
code the compiler emitted: every `#[inline(never)]` probe (`kernels/probe.rs`, symbols
`uop_probe_k_*`, and `exec/pe/probe.rs`, symbols `uop_probe_e_*`, test builds only) is the body
a handler pays for one op, with the `#[inline(always)]` kernels expanded into it. Per probe the
tool counts:

  insns     static instructions of the function (an upper bound of any path through it, because
            the probes have no loops; cold paths that were laid out inline are included)
  calls     `call` instructions (the cold reference paths and `Core::aot_note_sticky`)
  branches  conditional and unconditional jumps
  rets      `ret` instructions

Three inputs, one count. `--build` compiles the lib's test target with `cargo rustc` and
`--emit=asm` (one codegen unit and the release code shape for `nord-sharc` only: `opt-level=3`,
no overflow checks, no debug assertions; `--emit obj` emits an object instead) and counts the
instruction lines of the compiler's own assembly: no disassembler is involved, so the count is
made the same way on every target. `--asm FILE` reads such a file. An object file (or `--text`,
a listing of `objdump -d -r -C --no-show-raw-insn`, GNU or LLVM) goes through `objdump -d`; that
route is exact on ELF and Mach-O, but GNU objdump splits a COFF (MSVC) object at the
exception-funclet symbols inside a function and decodes the pieces as garbage, so a probe whose
listing has `(bad)` or `.byte` lines is refused (exit 2) instead of counted wrongly.

This is a count of instructions, never of time: the verdict is the same on every machine that
builds the same code with the same compiler.

Budget (docs/research/sharc-compiler-design.md section 12, gate K2): the paired average of the
element probe `uop_probe_e_favg2` is at most 260 x86 instructions per both-element op, target 150
to 200. The numbers are ASSUMED decision thresholds from the design (the spike's kernels), not
measurements; `--budget NAME=N` adds or replaces a limit. Gate K1 (block size) is measured on
whole blocks by `uop-report`, not here.

Exit code: 0 every checked budget held (or `--check` not given), 1 a budget was exceeded, 2 no
input, no probe found, a missing probe named by a budget, an unreadable listing or a failed tool.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
from collections.abc import Iterable
from dataclasses import dataclass, field
from pathlib import Path

SCHEMA = 1
#: Every probe symbol starts with this: `uop_probe_k_*` (kernel) and `uop_probe_e_*` (element).
PREFIX = "uop_probe_"
CRATE = "nord-sharc"

#: `objdump -d` function label: `0000000000000000 <symbol>:`.
FUNC_RE = re.compile(r"^[0-9a-fA-F]+ <(?P<label>.+)>:\s*$")
#: An instruction line: `   1c:<TAB>mov    %rax,%rbx` (an address, a colon, then a TAB). A
#: relocation line (`  1: IMAGE_REL_AMD64_REL32<TAB>sym`) has a space after the colon: no match.
INSN_RE = re.compile(r"^\s*[0-9a-fA-F]+:\t(?P<rest>.*)$")
#: A relocation line (`objdump -r`) naming the target of the call before it.
RELOC_RE = re.compile(r"^\s+[0-9a-fA-F]+:\s+(?:IMAGE_REL|R_X86_64|R_)\w*\s+(?P<sym>\S+)")
RAW_BYTES_RE = re.compile(r"[0-9a-f]{2}( [0-9a-f]{2})*\s*")
#: What objdump prints for bytes it could not decode as an instruction.
GARBAGE = ("(bad)", ".byte")
#: The 16-hex-digit hash the legacy Rust mangling appends to a path.
HASH_RE = re.compile(r"h[0-9a-f]{16}")
#: A label line of compiler assembly, not a local one (`.LBB0_1:`); the name may be quoted.
ASM_LABEL_RE = re.compile(r'^(?P<label>"[^"]+"|[^\s.#;"][^\s:"]*):')
#: An instruction line of compiler assembly: a TAB, a mnemonic, operands, a comment. A directive
#: starts with a dot (`.seh_proc`, `.p2align`), so it never matches.
ASM_INSN_RE = re.compile(r"^[ \t]+(?P<mn>[A-Za-z][\w.]*)(?:[ \t]+(?P<ops>[^#;]*))?")
#: The directives that end a function in compiler assembly: MSVC SEH, ELF/Mach-O CFI, and the
#: start of the next function's section (`.def` on COFF, `.section` everywhere).
ASM_END = (".seh_endproc", ".cfi_endproc", ".def", ".section")


@dataclass
class Probe:
    """The counts of one probe function."""

    name: str
    insns: int = 0
    calls: int = 0
    branches: int = 0
    rets: int = 0
    callees: list[str] = field(default_factory=list)

    def as_json(self) -> dict:
        return {
            "insns": self.insns,
            "calls": self.calls,
            "branches": self.branches,
            "rets": self.rets,
            "callees": self.callees,
        }

    def count(self, mnemonic: str) -> bool:
        """Counts one instruction; True when it is a call (its target follows)."""
        self.insns += 1
        if mnemonic.startswith("call"):
            self.calls += 1
            return True
        if mnemonic.startswith("j"):
            self.branches += 1
        elif mnemonic.startswith("ret"):
            self.rets += 1
        return False


@dataclass(frozen=True)
class Budget:
    """An instruction limit and, optionally, the range the design aims for."""

    limit: int
    target: tuple[int, int] | None = None


#: Gate K2 (design section 12): avg <= 260 instructions per both-element op, target 150-200.
BUDGETS = {"uop_probe_e_favg2": Budget(260, (150, 200))}


@dataclass(frozen=True)
class Verdict:
    """One checked budget: the probe, its count, the limit and the outcome."""

    name: str
    insns: int | None
    budget: Budget
    note: str
    ok: bool

    def line(self) -> str:
        mark = "PASS" if self.ok else "FAIL"
        count = "missing" if self.insns is None else str(self.insns)
        return f"{mark} {self.name} = {count} (limit {self.budget.limit}) {self.note}"


class ToolError(RuntimeError):
    """The input could not be produced or read (exit code 2)."""


def probe_name(label: str, prefix: str = PREFIX) -> str | None:
    """The probe symbol a function label names, or None for any other function.

    Handles a demangled path (`a::b::uop_probe_k_fadd`, optionally with a `::h<hash>` tail), a
    legacy-mangled one (`_ZN..17uop_probe_k_fadd17h<hash>E`) and a bare name.
    """
    parts: list[str]
    if label.startswith("_ZN"):
        parts, i = [], 3
        while i < len(label) and label[i].isdigit():
            j = i
            while label[j].isdigit():
                j += 1
            size = int(label[i:j])
            parts.append(label[j : j + size])
            i = j + size
    else:
        parts = label.split("::")
    if parts and HASH_RE.fullmatch(parts[-1]):
        parts.pop()
    name = parts[-1] if parts else ""
    return name if name.startswith(prefix) else None


def parse_listing(text: str, prefix: str = PREFIX) -> dict[str, Probe]:
    """Counts the probe functions of an `objdump -d` listing (see the module docs).

    A probe that appears twice (the same symbol in two objects) keeps its first count. A probe
    whose listing holds bytes objdump could not decode raises `ToolError`: its count is wrong.
    """
    probes: dict[str, Probe] = {}
    current: Probe | None = None
    last_call: Probe | None = None
    for line in text.splitlines():
        head = FUNC_RE.match(line)
        if head:
            name = probe_name(head["label"], prefix)
            current = Probe(name) if name and name not in probes else None
            if current:
                probes[name] = current
            last_call = None
            continue
        if current is None:
            continue
        insn = INSN_RE.match(line)
        if not insn:
            reloc = RELOC_RE.match(line)
            if reloc and last_call is not None:
                last_call.callees.append(reloc["sym"])
                last_call = None
            continue
        parts = insn["rest"].split("\t")
        # with raw bytes shown the line is `bytes<TAB>text`; a long encoding's continuation has
        # no text
        if RAW_BYTES_RE.fullmatch(parts[0]):
            parts = parts[1:]
        text_part = parts[0].strip() if parts else ""
        if not text_part:
            continue
        if text_part.startswith(GARBAGE):
            raise ToolError(
                f"{current.name}: objdump decoded data as code (a COFF object split at an "
                "exception funclet symbol); count the compiler assembly instead (--asm, --build)"
            )
        last_call = current if current.count(text_part.split()[0]) else None
    return probes


def parse_asm(lines: Iterable[str], prefix: str = PREFIX) -> dict[str, Probe]:
    """Counts the probe functions of a rustc `--emit=asm` file (see the module docs).

    A function runs from its label to the first end directive (`ASM_END`) or the next non-local
    label; the instruction lines in between are counted, directives and local labels are not. A
    probe that appears twice keeps its first count.
    """
    probes: dict[str, Probe] = {}
    current: Probe | None = None
    for line in lines:
        label = ASM_LABEL_RE.match(line)
        if label or line.lstrip().startswith(ASM_END):
            # a function ends at its end directive, at the next section or at the next label
            # (a function without unwind data, which has no `.seh_proc`, has no end directive)
            current = None
            name = probe_name(label["label"].strip('"'), prefix) if label else None
            if name and name not in probes:
                current = probes[name] = Probe(name)
            continue
        insn = ASM_INSN_RE.match(line) if current is not None else None
        if insn and current.count(insn["mn"]):
            current.callees.append((insn["ops"] or "").strip())
    return probes


def evaluate(probes: dict[str, Probe], budgets: dict[str, Budget]) -> list[Verdict]:
    """Checks every budget against the counted probes; a budget whose probe is absent fails."""
    out = []
    for name, budget in sorted(budgets.items()):
        probe = probes.get(name)
        if probe is None:
            out.append(Verdict(name, None, budget, "(probe not in the listing)", False))
            continue
        n, low_high = probe.insns, budget.target
        if n > budget.limit:
            note = "over the limit"
        elif low_high and n > low_high[1]:
            note = f"under the limit, above the target {low_high[0]}..{low_high[1]}"
        elif low_high and n < low_high[0]:
            note = f"below the target {low_high[0]}..{low_high[1]}"
        else:
            note = "within the target" if low_high else "within the limit"
        out.append(Verdict(name, n, budget, note, n <= budget.limit))
    return out


def format_table(probes: dict[str, Probe]) -> str:
    """One line per probe, sorted by name."""
    width = max([len(n) for n in probes] + [5])
    rows = [f"{'probe':<{width}}  {'insns':>6}  {'calls':>5}  {'branches':>8}  {'rets':>4}"]
    for name in sorted(probes):
        p = probes[name]
        rows.append(f"{name:<{width}}  {p.insns:>6}  {p.calls:>5}  {p.branches:>8}  {p.rets:>4}")
    return "\n".join(rows)


def find_objdump(explicit: str | None) -> str:
    """The objdump to run: the argument, `$OBJDUMP`, then `objdump` and `llvm-objdump`."""
    for cand in (explicit, os.environ.get("OBJDUMP"), "objdump", "llvm-objdump"):
        if cand and shutil.which(cand):
            return cand
    raise ToolError("no objdump found (pass --objdump PATH or set OBJDUMP)")


def disassemble(objdump: str, path: Path) -> str:
    """`objdump -d -r -C --no-show-raw-insn PATH` as text."""
    cmd = [objdump, "-d", "-r", "-C", "--no-show-raw-insn", str(path)]
    done = subprocess.run(cmd, capture_output=True, text=True, check=False)
    if done.returncode != 0:
        raise ToolError(f"{' '.join(cmd)} failed: {done.stderr.strip()[:300]}")
    return done.stdout


def build_command(emit: str = "asm") -> list[str]:
    """The cargo command that emits the optimized asm or object of the lib's test target."""
    pkg = f"profile.test.package.{CRATE}"
    return [
        "cargo",
        "rustc",
        "-p",
        CRATE,
        "--lib",
        "--profile",
        "test",
        "--config",
        f"{pkg}.opt-level=3",
        "--config",
        f"{pkg}.codegen-units=1",
        "--config",
        f"{pkg}.debug=0",
        "--config",
        f"{pkg}.overflow-checks=false",
        "--config",
        f"{pkg}.debug-assertions=false",
        "--",
        f"--emit={emit}",
    ]


def build_output(root: Path, emit: str = "asm") -> Path:
    """Builds the probe asm (or object) in the workspace `root` and returns the newest file."""
    done = subprocess.run(build_command(emit), cwd=root, check=False)
    if done.returncode != 0:
        raise ToolError("cargo rustc failed")
    ext = "s" if emit == "asm" else "o"
    target = Path(os.environ.get("CARGO_TARGET_DIR") or root / "target")
    found = sorted(
        target.glob(f"*/deps/{CRATE.replace('-', '_')}-*.{ext}"),
        key=lambda p: p.stat().st_mtime,
    )
    if not found:
        raise ToolError(f"cargo rustc produced no .{ext} file")
    return found[-1]


def parse_budget(arg: str) -> tuple[str, Budget]:
    """`NAME=N` to a limit without a target range."""
    name, sep, num = arg.partition("=")
    if not sep or not name or not num.isdigit():
        raise ToolError(f"--budget wants NAME=N, got {arg!r}")
    return name, Budget(int(num))


def read_asm(path: str, prefix: str) -> dict[str, Probe]:
    """The probes of an assembly file ('-' for standard input), read line by line."""
    if path == "-":
        return parse_asm(sys.stdin, prefix)
    with open(path, encoding="utf-8", errors="replace") as handle:
        return parse_asm(handle, prefix)


def collect(args: argparse.Namespace) -> dict[str, Probe]:
    """The probes of every input the command line names (the first count of a probe wins)."""
    found: list[dict[str, Probe]] = []
    objects = [Path(p) for p in args.inputs]
    if args.build:
        built = build_output(Path(__file__).resolve().parent.parent, args.emit)
        if args.emit == "asm":
            found.append(read_asm(str(built), args.prefix))
        else:
            objects.append(built)
    if args.asm:
        found.append(read_asm(args.asm, args.prefix))
    if args.text:
        text = sys.stdin.read() if args.text == "-" else Path(args.text).read_text("utf-8")
        found.append(parse_listing(text, args.prefix))
    if objects:
        objdump = find_objdump(args.objdump)
        found.extend(parse_listing(disassemble(objdump, p), args.prefix) for p in objects)
    if not (objects or args.build or args.asm or args.text):
        raise ToolError("no input: give an object file, --asm FILE, --text FILE or --build")
    probes: dict[str, Probe] = {}
    for one in found:
        for name, probe in one.items():
            probes.setdefault(name, probe)
    if not probes:
        raise ToolError(f"no function named {args.prefix}* in the input")
    return probes


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("inputs", nargs="*", help="object files or executables with symbols")
    ap.add_argument("--build", action="store_true", help="compile the test target and count it")
    ap.add_argument("--emit", choices=("asm", "obj"), default="asm", help="what --build emits")
    ap.add_argument("--asm", metavar="FILE", help="rustc --emit=asm output ('-' for stdin)")
    ap.add_argument("--text", metavar="FILE", help="an `objdump -d` listing ('-' for stdin)")
    ap.add_argument("--objdump", metavar="PATH", help="the objdump to run (default: PATH)")
    ap.add_argument("--prefix", default=PREFIX, help=f"probe symbol prefix (default {PREFIX})")
    ap.add_argument("--check", action="store_true", help="fail when a budget is exceeded")
    ap.add_argument("--budget", action="append", default=[], metavar="NAME=N", help="add a limit")
    ap.add_argument("--json", action="store_true", help="print JSON instead of a table")
    args = ap.parse_args(argv)
    try:
        budgets = dict(BUDGETS)
        budgets.update(parse_budget(b) for b in args.budget)
        probes = collect(args)
    except (ToolError, OSError) as err:
        print(f"uop_insn_count: {err}", file=sys.stderr)
        return 2
    verdicts = evaluate(probes, budgets) if args.check else []
    if args.json:
        doc = {
            "schema": SCHEMA,
            "probes": {n: p.as_json() for n, p in sorted(probes.items())},
            "checks": [
                {"name": v.name, "insns": v.insns, "limit": v.budget.limit, "ok": v.ok}
                for v in verdicts
            ],
        }
        print(json.dumps(doc, indent=2))
    else:
        print(format_table(probes))
        for v in verdicts:
            print(v.line())
    if any(v.insns is None for v in verdicts):
        return 2
    return 0 if all(v.ok for v in verdicts) else 1


if __name__ == "__main__":
    sys.exit(main())
