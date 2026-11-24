import bump_version as bv
import pytest
import release_package as rp

CARGO = (
    '[workspace]\nmembers = []\n\n[workspace.package]\nversion = "0.1.4"\n'
    'edition = "2021"\n\n[dependencies]\nversion = "9"\n'
)
PYPROJECT = '[project]\nname = "x"\nversion = "0.1.4"\n\n[tool.other]\nversion = "7"\n'


def make_root(tmp_path, pyproject=PYPROJECT):
    (tmp_path / "Cargo.toml").write_bytes(CARGO.encode())
    (tmp_path / "pyproject.toml").write_bytes(pyproject.encode())
    return tmp_path


def test_bump_rewrites_only_the_package_versions(tmp_path):
    root = make_root(tmp_path)
    assert bv.bump(root, "0.1.5") == "0.1.4"
    assert rp.workspace_version(root) == "0.1.5"
    assert 'version = "9"' in (root / "Cargo.toml").read_text()
    py = (root / "pyproject.toml").read_text()
    assert 'version = "0.1.5"' in py and 'version = "7"' in py


def test_bump_keeps_crlf(tmp_path):
    root = make_root(tmp_path, PYPROJECT.replace("\n", "\r\n"))
    bv.bump(root, "0.2.0-rc.1")
    assert b"\r\n" in (root / "pyproject.toml").read_bytes()


@pytest.mark.parametrize("bad", ["v0.1.5", "0.1", "1.2.3.4", "abc", ""])
def test_bump_rejects_malformed_versions(tmp_path, bad):
    with pytest.raises(ValueError):
        bv.bump(make_root(tmp_path), bad)


def test_bump_rejects_same_version_and_disagreeing_files(tmp_path):
    root = make_root(tmp_path)
    with pytest.raises(ValueError, match="already"):
        bv.bump(root, "0.1.4")
    (root / "pyproject.toml").write_text(PYPROJECT.replace("0.1.4", "0.1.3", 1))
    with pytest.raises(ValueError, match="disagree"):
        bv.bump(root, "0.1.5")


def test_main_no_lock_reports_error_code(tmp_path, monkeypatch, capsys):
    root = make_root(tmp_path)
    monkeypatch.setattr(bv, "ROOT", root)
    assert bv.main(["0.1.5", "--no-lock"]) == 0
    assert bv.main(["0.1.5", "--no-lock"]) == 1
    assert "already" in capsys.readouterr().err
