import io
import json
import os
import sys

import pytest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import uop_insn_count as t  # noqa: E402

#: A canned `objdump -d -r -C --no-show-raw-insn` listing: two probes (one calls a cold helper
#: through a relocation), a function that is not a probe and the label of a mangled probe.
LISTING = """\
crates.o:     file format coff-x86-64


Disassembly of section .text$uop_probe_k_fadd:

0000000000000000 <nord_sharc::core::uop::kernels::probe::uop_probe_k_fadd>:
   0:\tpush   %rbp
   1:\tmov    %rsp,%rbp
   4:\ttest   %rcx,%rcx
   7:\tje     12 <nord_sharc::core::uop::kernels::probe::uop_probe_k_fadd+0x12>
   9:\tcall   e <nord_sharc::core::uop::kernels::probe::uop_probe_k_fadd+0xe>
\t\t\ta: IMAGE_REL_AMD64_REL32\t_ZN10nord_sharc4core3uop7kernels3add4cold17h00112233445566778E
   e:\tmov    %rax,(%rdx)
  11:\tjmp    14 <nord_sharc::core::uop::kernels::probe::uop_probe_k_fadd+0x14>
  12:\tpop    %rbp
  13:\tret

0000000000000020 <nord_sharc::core::uop::exec::pe::probe::uop_probe_e_favg2>:
  20:\tmov    %rcx,%rax
  23:\tadd    $0x1,%rax
  27:\tret

0000000000000040 <nord_sharc::core::uop::exec::pe::exec_both>:
  40:\tnop
  41:\tnop
  42:\tret

0000000000000050 <_ZN10nord_sharc4core3uop7kernels5probe16uop_probe_k_fmul17h00aa00bb00cc00ddE>:
  50:\tmulsd  %xmm1,%xmm0
  54:\tret
"""

CALLEE = "_ZN10nord_sharc4core3uop7kernels3add4cold17h00112233445566778E"

#: Compiler assembly as rustc emits it, for the MSVC target (Intel syntax, SEH) then for ELF
#: (AT&T syntax, CFI): a probe with a call and a local label, a function that is not a probe,
#: a funclet label, a probe that ends in a tail jump and a repeated probe.
ASM = """\
\t.def\t_ZN10nord_sharc4core3uop7kernels5probe16uop_probe_k_fadd17h0123456789abcdefE;
\t.scl\t2;
\t.type\t32;
\t.endef
\t.globl\t_ZN10nord_sharc4core3uop7kernels5probe16uop_probe_k_fadd17h0123456789abcdefE
\t.p2align\t4
_ZN10nord_sharc4core3uop7kernels5probe16uop_probe_k_fadd17h0123456789abcdefE:
.seh_proc _ZN10nord_sharc4core3uop7kernels5probe16uop_probe_k_fadd17h0123456789abcdefE
# %bb.0:
\tpush\trbp
\t.seh_pushreg\trbp
\tmov\trbp, rsp
\ttest\trcx, rcx
\tje\t.LBB0_2
# %bb.1:
\tcall\t_ZN10nord_sharc4core3uop7kernels3add4cold17h00112233445566778E
.LBB0_2:
\tmov\tqword ptr [rdx], rax # a comment: not an instruction
\tpop\trbp
\tret
\t.seh_endproc

_ZN10nord_sharc4core3uop4lift4lift17h00112233445566778E:
\tnop
\tnop
\tret
\t.seh_endproc
"?dtor$2@?0?_ZN4core3ptr13drop_in_place17h1cb36c0e8921c5b7E@4HA":
\tmov\trax, rax
\t.seh_endproc

\t.type\tuop_probe_e_favg2,@function
uop_probe_e_favg2:
\t.cfi_startproc
\tmovq\t%rdi, %rax
\taddq\t$1, %rax
\tjmp\tother_fn@PLT
\t.cfi_endproc
\t.size\tuop_probe_e_favg2, .-uop_probe_e_favg2

\t.def\t_ZN10nord_sharc4core3uop7kernels5probe16uop_probe_k_fcmp17h0123456789abcdefE;
\t.scl\t3;
\t.type\t32;
\t.endef
\t.section\t.text,"xr",one_only,_ZN10nord_sharc4core3uop7kernels5probe16uop_probe_k_fcmp17h0123456789abcdefE,unique,7
\t.p2align\t4
_ZN10nord_sharc4core3uop7kernels5probe16uop_probe_k_fcmp17h0123456789abcdefE:
\tmovq\t%rcx, %rax
\tcmpq\t%rdx, %rax
\tsetl\t%al
\tretq

\t.def\t_ZN10nord_sharc4core3uop7kernels5probe16uop_probe_k_ffix17h0123456789abcdefE;
\t.scl\t3;
\t.type\t32;
\t.endef
\t.section\t.text,"xr",one_only,_ZN10nord_sharc4core3uop7kernels5probe16uop_probe_k_ffix17h0123456789abcdefE,unique,8
\t.p2align\t4
_ZN10nord_sharc4core3uop7kernels5probe16uop_probe_k_ffix17h0123456789abcdefE:
\tmovl\t%r9d, %eax
\tjmp\t_ZN10nord_sharc4core3uop7kernels7convert4ffix17h666eba9bbb06b4a1E

uop_probe_e_favg2:
\tnop
\t.cfi_endproc
"""


def test_the_probe_functions_are_counted_and_other_functions_are_ignored():
    probes = t.parse_listing(LISTING)
    assert sorted(probes) == ["uop_probe_e_favg2", "uop_probe_k_fadd", "uop_probe_k_fmul"]
    fadd = probes["uop_probe_k_fadd"]
    assert (fadd.insns, fadd.calls, fadd.branches, fadd.rets) == (9, 1, 2, 1)
    assert (probes["uop_probe_e_favg2"].insns, probes["uop_probe_e_favg2"].rets) == (3, 1)
    assert probes["uop_probe_k_fmul"].insns == 2, "a legacy-mangled label is demangled by hand"


def test_a_relocation_line_names_the_callee_and_is_not_an_instruction():
    fadd = t.parse_listing(LISTING)["uop_probe_k_fadd"]
    assert fadd.callees == [CALLEE]
    # the relocation line sits between two instructions and changes no count
    assert fadd.insns == 9


def test_a_listing_with_raw_bytes_counts_the_same_and_skips_continuation_lines():
    raw = (
        "0000000000000000 <uop_probe_k_fcmp>:\n"
        "   0:\t55                   \tpush   %rbp\n"
        "   1:\t48 b8 01 02 03 04 05 \tmovabs $0x0807060504030201,%rax\n"
        "   8:\t06 07 08\n"
        "   b:\tc3                   \tret\n"
    )
    probe = t.parse_listing(raw)["uop_probe_k_fcmp"]
    assert (probe.insns, probe.rets) == (3, 1)


def test_probe_name_reads_demangled_mangled_and_bare_labels():
    p = t.probe_name
    assert p("a::b::uop_probe_k_fadd") == "uop_probe_k_fadd"
    assert p("a::b::uop_probe_k_fadd::h0123456789abcdef") == "uop_probe_k_fadd"
    assert p("_ZN1a1b16uop_probe_k_fadd17h0123456789abcdefE") == "uop_probe_k_fadd"
    assert p("uop_probe_e_ffix") == "uop_probe_e_ffix"
    assert p("a::b::exec_both") is None
    assert p("_ZN1a1b9exec_both17h0123456789abcdefE") is None
    assert p("uop_probe_x_y", prefix="other_") is None


def test_a_probe_that_appears_twice_keeps_its_first_count():
    twice = (
        "0 <uop_probe_k_fadd>:\n   0:\tnop\n   1:\tret\n"
        "10 <uop_probe_k_fadd>:\n  10:\tnop\n  11:\tnop\n  12:\tnop\n  13:\tret\n"
    )
    probes = t.parse_listing(twice)
    assert list(probes) == ["uop_probe_k_fadd"]
    assert probes["uop_probe_k_fadd"].insns == 2


def test_asm_counts_instruction_lines_only_between_a_probe_label_and_its_end_directive():
    probes = t.parse_asm(ASM.splitlines())
    assert sorted(probes) == [
        "uop_probe_e_favg2",
        "uop_probe_k_fadd",
        "uop_probe_k_fcmp",
        "uop_probe_k_ffix",
    ]
    fadd = probes["uop_probe_k_fadd"]
    # directives, comments, labels and the local label are not instructions; the comment on the
    # `mov` line does not hide it
    assert (fadd.insns, fadd.calls, fadd.branches, fadd.rets) == (8, 1, 1, 1)
    assert fadd.callees == [CALLEE]
    favg = probes["uop_probe_e_favg2"]
    assert (favg.insns, favg.calls, favg.branches, favg.rets) == (3, 0, 1, 0)
    assert favg.insns == 3, "the second function of the same name is ignored"


def test_asm_ends_a_function_without_unwind_data_at_the_next_section_or_label():
    """A leaf function has no `.seh_proc`/`.seh_endproc`: the next function's `.def` ends it."""
    probes = t.parse_asm(ASM.splitlines())
    fcmp, ffix = probes["uop_probe_k_fcmp"], probes["uop_probe_k_ffix"]
    assert (fcmp.insns, fcmp.rets, fcmp.branches) == (4, 1, 0)
    # a tail jump is a branch, not a call, and the next function's lines are not counted
    assert (ffix.insns, ffix.calls, ffix.branches, ffix.rets) == (2, 0, 1, 0)
    # a label alone ends a function too
    bare = ["uop_probe_k_a:", "\tnop", "\tretq", "other:", "\tnop", "uop_probe_k_b:", "\tretq"]
    counted = {n: p.insns for n, p in t.parse_asm(bare).items()}
    assert counted == {"uop_probe_k_a": 2, "uop_probe_k_b": 1}


def test_asm_ignores_a_function_that_is_not_a_probe_and_a_funclet_label():
    assert "lift" not in "".join(t.parse_asm(ASM.splitlines()))
    only_other = ["other:", "\tnop", "\tret", "\t.seh_endproc"]
    assert t.parse_asm(only_other) == {}


def test_budgets_pass_inside_the_target_and_under_the_limit_and_fail_over_it():
    budget = {"uop_probe_e_favg2": t.Budget(260, (150, 200))}

    def check(n):
        probes = {"uop_probe_e_favg2": t.Probe("uop_probe_e_favg2", insns=n)}
        (v,) = t.evaluate(probes, budget)
        return v

    assert (check(180).ok, check(180).note) == (True, "within the target")
    assert check(230).ok and "above the target" in check(230).note
    assert check(100).ok and "below the target" in check(100).note
    assert check(260).ok
    assert not check(261).ok and check(261).note == "over the limit"
    assert "FAIL" in check(261).line() and "PASS" in check(180).line()


def test_a_budget_without_its_probe_fails_and_says_so():
    (v,) = t.evaluate({}, {"uop_probe_e_favg2": t.Budget(260)})
    assert not v.ok and v.insns is None and "missing" in v.line()


def test_the_default_budget_is_the_k2_average_limit():
    b = t.BUDGETS["uop_probe_e_favg2"]
    assert (b.limit, b.target) == (260, (150, 200))


def test_budget_arguments_are_name_equals_number():
    assert t.parse_budget("uop_probe_k_fadd=60") == ("uop_probe_k_fadd", t.Budget(60))
    for bad in ("uop_probe_k_fadd", "=5", "x=", "x=five", "x=-3"):
        with pytest.raises(t.ToolError):
            t.parse_budget(bad)


def test_the_table_lists_every_probe_with_its_counts():
    table = t.format_table(t.parse_listing(LISTING))
    lines = table.splitlines()
    assert lines[0].split() == ["probe", "insns", "calls", "branches", "rets"]
    assert [ln.split()[0] for ln in lines[1:]] == [
        "uop_probe_e_favg2",
        "uop_probe_k_fadd",
        "uop_probe_k_fmul",
    ]
    assert lines[2].split() == ["uop_probe_k_fadd", "9", "1", "2", "1"]


def test_the_build_command_emits_one_optimized_unit_of_the_test_target():
    cmd = t.build_command()
    assert cmd[:3] == ["cargo", "rustc", "-p"] and "nord-sharc" in cmd
    assert "--lib" in cmd and cmd[cmd.index("--profile") + 1] == "test"
    assert cmd[-2:] == ["--", "--emit=asm"], "the default is the compiler's own assembly"
    assert t.build_command("obj")[-1] == "--emit=obj"
    configs = [cmd[i + 1] for i, a in enumerate(cmd) if a == "--config"]
    assert "profile.test.package.nord-sharc.opt-level=3" in configs
    assert "profile.test.package.nord-sharc.codegen-units=1" in configs
    # the release code shape: the test profile's overflow checks and assertions would add code
    assert "profile.test.package.nord-sharc.overflow-checks=false" in configs
    assert "profile.test.package.nord-sharc.debug-assertions=false" in configs


def run(tmp_path, capsys, listing, *argv):
    path = tmp_path / "listing.txt"
    path.write_text(listing, encoding="utf-8")
    code = t.main(["--text", str(path), *argv])
    return code, capsys.readouterr()


def test_main_prints_the_table_and_checks_the_budget(tmp_path, capsys):
    code, out = run(tmp_path, capsys, LISTING)
    assert code == 0 and "uop_probe_k_fadd" in out.out and "PASS" not in out.out
    # without --check no budget is evaluated; with it the favg2 probe is 3 instructions:
    # below the 150..200 target but under the limit, so it passes
    code, out = run(tmp_path, capsys, LISTING, "--check")
    assert code == 0 and "PASS uop_probe_e_favg2 = 3 (limit 260) below the target" in out.out


def test_main_fails_with_one_when_a_budget_is_exceeded(tmp_path, capsys):
    code, out = run(tmp_path, capsys, LISTING, "--check", "--budget", "uop_probe_k_fadd=8")
    assert code == 1
    assert "FAIL uop_probe_k_fadd = 9 (limit 8) over the limit" in out.out


def test_main_exits_with_two_when_the_input_is_unusable(tmp_path, capsys):
    code, out = run(tmp_path, capsys, "0 <other>:\n   0:\tret\n")
    assert code == 2 and "no function named uop_probe_" in out.err
    code, out = run(tmp_path, capsys, LISTING, "--check", "--budget", "uop_probe_k_gone=5")
    assert code == 2, "a budget for a probe that is not there is not a pass"
    assert "FAIL uop_probe_k_gone = missing" in out.out
    assert t.main([]) == 2 and "no input" in capsys.readouterr().err
    assert t.main(["--text", str(tmp_path / "absent.txt")]) == 2
    assert t.main(["--text", "x", "--budget", "bad"]) == 2


def test_main_json_carries_every_probe_and_the_checks(tmp_path, capsys):
    code, out = run(tmp_path, capsys, LISTING, "--check", "--json")
    doc = json.loads(out.out)
    assert code == 0 and doc["schema"] == t.SCHEMA
    assert doc["probes"]["uop_probe_k_fadd"] == {
        "insns": 9,
        "calls": 1,
        "branches": 2,
        "rets": 1,
        "callees": [CALLEE],
    }
    assert doc["checks"] == [{"name": "uop_probe_e_favg2", "insns": 3, "limit": 260, "ok": True}]


def test_a_counted_listing_changes_when_an_instruction_is_added():
    """The count follows the listing: one more instruction in a probe is one more counted."""
    longer = LISTING.replace("  27:\tret\n", "  27:\tnop\n  28:\tret\n")
    assert longer != LISTING
    before = t.parse_listing(LISTING)["uop_probe_e_favg2"].insns
    after = t.parse_listing(longer)["uop_probe_e_favg2"].insns
    assert (before, after) == (3, 4)


def test_main_reads_a_listing_from_standard_input(monkeypatch, capsys):
    monkeypatch.setattr(sys, "stdin", io.StringIO(LISTING))
    assert t.main(["--text", "-"]) == 0
    assert "uop_probe_k_fmul" in capsys.readouterr().out


def test_main_counts_an_asm_file_and_reads_asm_from_standard_input(tmp_path, capsys, monkeypatch):
    path = tmp_path / "probe.s"
    path.write_text(ASM, encoding="utf-8")
    assert t.main(["--asm", str(path), "--json"]) == 0
    doc = json.loads(capsys.readouterr().out)
    assert doc["probes"]["uop_probe_k_fadd"]["insns"] == 8
    monkeypatch.setattr(sys, "stdin", io.StringIO(ASM))
    assert t.main(["--asm", "-"]) == 0
    assert "uop_probe_e_favg2" in capsys.readouterr().out


def test_an_objdump_listing_with_undecoded_bytes_is_refused_not_miscounted(tmp_path, capsys):
    """GNU objdump splits a COFF function at a funclet symbol and decodes the rest as garbage."""
    for junk in ("(bad)", ".byte 0xf"):
        listing = f"0 <uop_probe_k_fadd>:\n   0:\tpush   %rbp\n   1:\t{junk}\n   2:\tret\n"
        with pytest.raises(t.ToolError, match="decoded data as code"):
            t.parse_listing(listing)
        code, out = run(tmp_path, capsys, listing)
        assert code == 2 and "--asm" in out.err
    # a function that is not a probe may hold anything: it is not counted
    other = "0 <helper>:\n   0:\t(bad)\n10 <uop_probe_k_fadd>:\n  10:\tret\n"
    assert t.parse_listing(other)["uop_probe_k_fadd"].insns == 1


def test_inputs_are_merged_and_the_first_count_of_a_probe_wins(tmp_path, capsys):
    path = tmp_path / "probe.s"
    path.write_text(ASM, encoding="utf-8")
    code, out = run(tmp_path, capsys, LISTING, "--asm", str(path), "--json")
    probes = json.loads(out.out)["probes"]
    assert code == 0
    # `--asm` is read first: its 8-instruction fadd wins over the listing's 9
    assert probes["uop_probe_k_fadd"]["insns"] == 8
    assert probes["uop_probe_k_fmul"]["insns"] == 2, "a probe only the listing has is kept"
