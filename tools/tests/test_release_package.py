import gzip
import tarfile
import zipfile

import pytest
import release_package as rp

WIN = rp.PLATFORMS["x86_64-pc-windows-msvc"]
LIN = rp.PLATFORMS["x86_64-unknown-linux-gnu"]
MAC = rp.PLATFORMS["aarch64-apple-darwin"]


def make_tree(tmp_path, platform):
    """Synthetic repo + target dir with every file `collect` expects, plus stray junk."""
    root = tmp_path / "repo"
    (root / "docs").mkdir(parents=True)
    (root / "docs" / "RELEASE_README.txt").write_text("readme")
    (root / "docs" / "HOST_COMPATIBILITY.md").write_text("compat")
    (root / "Cargo.toml").write_text(
        '[workspace]\nmembers = []\n\n[workspace.package]\nversion = "1.2.3"\n'
    )
    rel = root / "target" / platform.triple / "release"
    rel.mkdir(parents=True)
    for name in rp.BINS:
        (rel / f"{name}{platform.exe}").write_bytes(b"bin-" + name.encode())
    (rel / "stray.ns4b").write_bytes(b"must never ship")
    bundled = root / "target" / "bundled"
    vst3 = bundled / "nd-matrix-1.vst3" / platform.vst3_binary
    vst3.parent.mkdir(parents=True)
    vst3.write_bytes(b"vst3")
    if platform.os == "macos":
        clap = bundled / "nd-matrix-1.clap" / "Contents" / "MacOS" / "nd-matrix-1"
        clap.parent.mkdir(parents=True)
        clap.write_bytes(b"clap")
    else:
        (bundled / "nd-matrix-1.clap").write_bytes(b"clap")
    (root / ".reference").mkdir()
    (root / ".reference" / "fw.bin").write_bytes(b"firmware")
    return root


def test_workspace_version_reads_workspace_package(tmp_path):
    root = make_tree(tmp_path, LIN)
    assert rp.workspace_version(root) == "1.2.3"


def test_workspace_version_missing_raises(tmp_path):
    (tmp_path / "Cargo.toml").write_text('[package]\nversion = "9.9.9"\n')
    with pytest.raises(ValueError):
        rp.workspace_version(tmp_path)


def test_real_workspace_version_is_semver():
    assert rp.TAG_RE.match("v" + rp.workspace_version())


@pytest.mark.parametrize("tag", ["0.1.0", "v1.2", "v1.2.3.4", "vx.y.z", "v1.2.3-"])
def test_bad_tags_rejected(tag):
    with pytest.raises(ValueError, match="not vMAJOR"):
        rp.version_from_tag(tag, "1.2.3")


def test_tag_must_match_cargo_version():
    with pytest.raises(ValueError, match="does not match"):
        rp.version_from_tag("v1.2.4", "1.2.3")
    assert rp.version_from_tag("v1.2.3", "1.2.3") == "1.2.3"
    assert rp.version_from_tag("v1.3.0-rc.1", "1.3.0-rc.1") == "1.3.0-rc.1"


def test_archive_stem():
    assert rp.archive_stem("v1.0.0", WIN) == "nd-matrix-1-v1.0.0-windows-x86_64"


def test_collect_excludes_stray_files(tmp_path):
    root = make_tree(tmp_path, LIN)
    names = {rel for _, rel in rp.collect(LIN, root)}
    assert "ns4" in names and "nd-matrix-1.clap" in names
    assert "nd-matrix-1.vst3/Contents/x86_64-linux/nd-matrix-1.so" in names
    assert not any("stray" in n or "fw.bin" in n or ".reference" in n for n in names)


def test_collect_missing_output_raises(tmp_path):
    root = make_tree(tmp_path, LIN)
    (root / "target" / "bundled" / "nd-matrix-1.clap").unlink()
    with pytest.raises(FileNotFoundError, match="nd-matrix-1.clap"):
        rp.collect(LIN, root)


def test_collect_includes_license_when_present(tmp_path):
    root = make_tree(tmp_path, LIN)
    (root / "LICENSE-MIT").write_text("mit")
    assert "LICENSE-MIT" in {rel for _, rel in rp.collect(LIN, root)}


def test_write_sha256_is_lf_and_sha256sum_format(tmp_path):
    archive = tmp_path / "a.zip"
    archive.write_bytes(b"abc")
    raw = rp.write_sha256(archive).read_bytes()
    assert raw == b"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  a.zip\n"


def test_package_zip_uses_forward_slashes_and_top_folder(tmp_path):
    root = make_tree(tmp_path, WIN)
    archive = rp.package(WIN.triple, "v1.2.3", tmp_path / "out", root)
    assert archive.name == "nd-matrix-1-v1.2.3-windows-x86_64.zip"
    with zipfile.ZipFile(archive) as zf:
        names = zf.namelist()
    stem = archive.name[: -len(".zip")]
    assert all(n.startswith(stem + "/") and "\\" not in n for n in names)
    assert f"{stem}/nd-matrix-1.vst3/Contents/x86_64-win/nd-matrix-1.vst3" in names
    assert f"{stem}/ns4.exe" in names
    assert archive.with_name(archive.name + ".sha256").exists()


def test_package_tar_sets_exec_bits_and_is_deterministic(tmp_path):
    root = make_tree(tmp_path, LIN)
    first = rp.package(LIN.triple, "v1.2.3", tmp_path / "o1", root)
    second = rp.package(LIN.triple, "v1.2.3", tmp_path / "o2", root)
    assert first.read_bytes() == second.read_bytes()
    with tarfile.open(first, "r:gz") as tf:
        modes = {m.name.split("/", 1)[1]: m.mode for m in tf.getmembers()}
    assert modes["ns4"] == 0o755
    assert modes["nd-matrix-1.clap"] == 0o755
    assert modes["nd-matrix-1.vst3/Contents/x86_64-linux/nd-matrix-1.so"] == 0o755
    assert modes["README.txt"] == 0o644
    assert gzip.open(first).read(1)


def test_macos_bundle_layout_packaged(tmp_path):
    root = make_tree(tmp_path, MAC)
    archive = rp.package(MAC.triple, "v1.2.3", tmp_path / "out", root)
    with tarfile.open(archive, "r:gz") as tf:
        modes = {m.name.split("/", 1)[1]: m.mode for m in tf.getmembers()}
    assert modes["nd-matrix-1.clap/Contents/MacOS/nd-matrix-1"] == 0o755
    assert modes["nd-matrix-1.vst3/Contents/MacOS/nd-matrix-1"] == 0o755


def test_main_dry_run_label_is_dev(tmp_path, capsys):
    root = make_tree(tmp_path, LIN)
    code = rp.main(["--target", LIN.triple, "--out", str(tmp_path / "o"), "--root", str(root)])
    assert code == 0
    assert "v1.2.3-dev-linux-x86_64.tar.gz" in capsys.readouterr().out


def test_main_rejects_mismatched_tag(tmp_path, capsys):
    root = make_tree(tmp_path, LIN)
    code = rp.main(
        ["--target", LIN.triple, "--tag", "v9.9.9", "--out", str(tmp_path), "--root", str(root)]
    )
    assert code == 1
    assert "does not match" in capsys.readouterr().err


def test_main_reports_missing_output(tmp_path, capsys):
    root = make_tree(tmp_path, LIN)
    (root / "target" / "bundled" / "nd-matrix-1.clap").unlink()
    code = rp.main(["--target", LIN.triple, "--out", str(tmp_path / "o"), "--root", str(root)])
    assert code == 1
    assert "missing build output" in capsys.readouterr().err


def test_check_tag_mode(tmp_path, capsys):
    root = make_tree(tmp_path, LIN)
    assert rp.main(["--check-tag", "v1.2.3", "--root", str(root)]) == 0
    assert "matches workspace version" in capsys.readouterr().out
    assert rp.main(["--check-tag", "v1.2.4", "--root", str(root)]) == 1
    assert "does not match" in capsys.readouterr().err


def test_target_required_without_check_tag(tmp_path):
    with pytest.raises(SystemExit):
        rp.main(["--root", str(tmp_path)])


def test_pyproject_version_tracks_workspace_version():
    import re

    text = (rp.ROOT / "pyproject.toml").read_text(encoding="utf-8")
    match = re.search(r'^version\s*=\s*"([^"]+)"', text, re.M)
    assert match and match.group(1) == rp.workspace_version()


def test_bundler_toml_names_the_plugin_bundle_after_plugin_stem():
    """`cargo xtask bundle` names bundles after bundler.toml `[nord-plugin] name`. It must equal
    PLUGIN_STEM or the packager fails with "missing build output" (v0.1.2 release)."""
    import tomllib

    path = rp.ROOT / "bundler.toml"
    renamed = tomllib.loads(path.read_text(encoding="utf-8"))["nord-plugin"]["name"]
    assert renamed == rp.PLUGIN_STEM == "nd-matrix-1"


def test_legacy_plugin_stem_is_the_old_bundle_name():
    assert rp.LEGACY_PLUGIN_STEM == "nord-plugin"
    assert rp.LEGACY_PLUGIN_STEM != rp.PLUGIN_STEM


def test_workspace_contact_metadata_has_no_foreign_email_and_current_repo_url():
    import tomllib

    pkg = tomllib.loads((rp.ROOT / "Cargo.toml").read_text(encoding="utf-8"))["workspace"][
        "package"
    ]
    assert pkg["authors"] == ["ymatrix1"]
    assert pkg["repository"] == "https://github.com/ymatrix1/nd-matrix-1"
