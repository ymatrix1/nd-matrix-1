import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import check_dco as c  # noqa: E402

SIGNED = "feat: x\n\nbody\n\nSigned-off-by: Ada Lovelace <ada@example.com>\n"


def test_has_signoff_accepts_trailer_before_other_trailers():
    msg = SIGNED + "Co-Authored-By: Someone <s@example.com>\n"
    assert c.has_signoff(msg)


def test_has_signoff_rejects_missing_or_malformed():
    assert not c.has_signoff("feat: x\n\nno trailer\n")
    assert not c.has_signoff("Signed-off-by: Ada\n")
    assert not c.has_signoff("Signed-off-by:\n")
    assert not c.has_signoff("see Signed-off-by: Ada <a@b.c> inline\n")


def test_missing_signoff_lists_unsigned_non_merge_commits():
    commits = [("a1", 1, SIGNED), ("b2", 1, "fix: y\n"), ("c3", 1, "chore: z\n")]
    assert c.missing_signoff(commits) == ["b2", "c3"]


def test_merge_commits_are_exempt():
    commits = [("m1", 2, "Merge branch 'x'\n"), ("r0", 0, SIGNED)]
    assert c.missing_signoff(commits) == []


def test_main_usage_error_returns_2(capsys):
    assert c.main(["check_dco.py"]) == 2
    assert "Signed-off-by" in capsys.readouterr().out


def test_main_uses_git_range(monkeypatch, capsys):
    monkeypatch.setattr(c, "read_range", lambda r: [("abc", 1, "x\n")])
    assert c.main(["check_dco.py", "base", "head"]) == 1
    assert "abc" in capsys.readouterr().out
    monkeypatch.setattr(c, "read_range", lambda r: [("abc", 1, SIGNED)])
    assert c.main(["check_dco.py", "base..head"]) == 0
