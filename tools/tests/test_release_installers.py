import plistlib

import pytest
import release_installers as ri
from test_release_package import LIN, MAC, WIN, make_tree


def _make_tree_with_licenses(tmp_path, platform):
    root = make_tree(tmp_path, platform)
    (root / "LICENSE").write_text("PolyForm Noncommercial 1.0.0", encoding="utf-8")
    (root / "LICENSE-PLUGIN").write_text("GNU GPL v3", encoding="utf-8")
    return root


def test_debian_and_numeric_versions():
    assert ri.debian_version("v0.1.0") == "0.1.0"
    assert ri.debian_version("v0.1.0-rc.1") == "0.1.0~rc.1"
    assert ri.debian_version("v0.1.0-dev") == "0.1.0~dev"
    assert ri.numeric_version("v1.2.3-rc.1") == "1.2.3"
    with pytest.raises(ValueError, match="numeric"):
        ri.numeric_version("bad-tag")


def test_stage_windows_inno_layout_and_iss(tmp_path):
    root = _make_tree_with_licenses(tmp_path, WIN)
    out_dir = tmp_path / "dist"
    work_dir = tmp_path / "work"
    iss_path, exe_path = ri.stage_windows_inno("v1.2.3", out_dir, work_dir, root=root)

    assert exe_path.name == "nd-matrix-1-v1.2.3-windows-x86_64-setup.exe"
    iss = iss_path.read_text(encoding="utf-8")
    assert 'Name: "app"' in iss
    assert 'Name: "cli"' in iss
    assert 'Name: "vst3"' in iss
    assert 'Name: "clap"' in iss
    assert r"{commoncf64}\VST3\nord-plugin.vst3" in iss
    assert r"{commoncf64}\CLAP" in iss
    assert "VersionInfoVersion=1.2.3" in iss
    assert "LicenseFile=" in iss
    assert "AppSupportURL" not in iss

    stage_dir = work_dir / "win-inno-stage"
    assert (stage_dir / "ns4.exe").exists()
    assert (stage_dir / "nd-matrix-1.exe").exists()
    assert (stage_dir / "nord-plugin.clap").exists()
    assert (
        stage_dir / "nord-plugin.vst3" / "Contents" / "x86_64-win" / "nord-plugin.vst3"
    ).exists()
    assert not (stage_dir / "stray.ns4b").exists()


def test_stage_linux_deb_layout_and_control(tmp_path):
    root = _make_tree_with_licenses(tmp_path, LIN)
    out_dir = tmp_path / "dist"
    work_dir = tmp_path / "work"
    pkg_root, deb_path = ri.stage_linux_deb("v1.2.3-rc.1", out_dir, work_dir, root=root)

    assert deb_path.name == "nd-matrix-1_1.2.3~rc.1_amd64.deb"
    control = (pkg_root / "DEBIAN" / "control").read_text(encoding="utf-8")
    assert "Package: nd-matrix-1\n" in control
    assert "Version: 1.2.3~rc.1\n" in control
    assert "Architecture: amd64\n" in control

    assert (pkg_root / "usr" / "bin" / "ns4").exists()
    assert (pkg_root / "usr" / "bin" / "nd-matrix-1").exists()
    assert (pkg_root / "usr" / "bin" / "nord-plugin-standalone").exists()
    assert (pkg_root / "usr" / "lib" / "clap" / "nord-plugin.clap").exists()
    assert (
        pkg_root
        / "usr"
        / "lib"
        / "vst3"
        / "nord-plugin.vst3"
        / "Contents"
        / "x86_64-linux"
        / "nord-plugin.so"
    ).exists()
    assert (pkg_root / "usr" / "share" / "applications" / "nd-matrix-1.desktop").exists()
    assert (pkg_root / "usr" / "share" / "doc" / "nd-matrix-1" / "LICENSE-PLUGIN").exists()


def test_stage_linux_appdir(tmp_path):
    root = _make_tree_with_licenses(tmp_path, LIN)
    appdir, appimage_path = ri.stage_linux_appdir(
        "v1.2.3", tmp_path / "dist", tmp_path / "work", root=root
    )
    assert appimage_path.name == "nd-matrix-1-v1.2.3-linux-x86_64.AppImage"
    apprun = (appdir / "AppRun").read_text(encoding="utf-8")
    assert "--install-plugins" in apprun
    assert (appdir / "nd-matrix-1.desktop").exists()
    assert (appdir / "nd-matrix-1.svg").exists()
    assert (appdir / "usr" / "bin" / "nd-matrix-1").exists()
    assert (appdir / "usr" / "lib" / "clap" / "nord-plugin.clap").exists()
    assert (
        appdir
        / "usr"
        / "lib"
        / "vst3"
        / "nord-plugin.vst3"
        / "Contents"
        / "x86_64-linux"
        / "nord-plugin.so"
    ).exists()


def test_stage_macos_dmg_and_app_bundle(tmp_path):
    root = _make_tree_with_licenses(tmp_path, MAC)
    dmg_stage, dmg_path = ri.stage_macos_dmg(
        "v1.2.3",
        tmp_path / "dist",
        tmp_path / "work",
        root=root,
        create_symlinks=False,
    )
    assert dmg_path.name == "nd-matrix-1-v1.2.3-macos-arm64.dmg"
    app_dir = dmg_stage / "ND Matrix 1.app"
    assert (app_dir / "Contents" / "MacOS" / "nd-matrix-1").exists()
    with (app_dir / "Contents" / "Info.plist").open("rb") as handle:
        plist = plistlib.load(handle)
    assert plist["CFBundleIdentifier"] == "org.ndmatrix.nd-matrix-1.app"
    assert plist["CFBundleExecutable"] == "nd-matrix-1"
    assert plist["CFBundleShortVersionString"] == "1.2.3"
    assert (dmg_stage / "nord-plugin.vst3" / "Contents" / "MacOS" / "nord-plugin").exists()
    assert (dmg_stage / "nord-plugin.clap" / "Contents" / "MacOS" / "nord-plugin").exists()
    assert (dmg_stage / "ns4").exists()
    assert (dmg_stage / "LICENSE-PLUGIN").exists()


def test_stage_macos_pkg_roots_and_distribution_xml(tmp_path):
    root = _make_tree_with_licenses(tmp_path, MAC)
    paths = ri.stage_macos_pkg_roots("v1.2.3", tmp_path / "dist", tmp_path / "work", root=root)
    assert paths["pkg_path"].name == "nd-matrix-1-v1.2.3-macos-arm64.pkg"
    assert (
        paths["app_root"] / "Applications" / "ND Matrix 1.app" / "Contents" / "Info.plist"
    ).exists()
    assert (paths["app_root"] / "usr" / "local" / "bin" / "ns4").exists()
    assert (
        paths["vst3_root"]
        / "Library"
        / "Audio"
        / "Plug-Ins"
        / "VST3"
        / "nord-plugin.vst3"
        / "Contents"
        / "MacOS"
        / "nord-plugin"
    ).exists()
    assert (
        paths["clap_root"]
        / "Library"
        / "Audio"
        / "Plug-Ins"
        / "CLAP"
        / "nord-plugin.clap"
        / "Contents"
        / "MacOS"
        / "nord-plugin"
    ).exists()
    xml = paths["dist_xml"].read_text(encoding="utf-8")
    assert 'id="choice-app"' in xml
    assert 'id="choice-vst3"' in xml
    assert 'id="choice-clap"' in xml


def test_main_dispatches_per_platform_formats(tmp_path, monkeypatch, capsys):
    root = _make_tree_with_licenses(tmp_path, WIN)
    calls = []

    def fake_win(label, out_dir, work_dir, r, td):
        calls.append(("win", label))
        p = out_dir / "fake-setup.exe"
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_bytes(b"exe")
        return p

    monkeypatch.setattr(ri, "build_windows_installer", fake_win)
    code = ri.main(
        [
            "--target",
            WIN.triple,
            "--out",
            str(tmp_path / "dist"),
            "--work",
            str(tmp_path / "work"),
            "--root",
            str(root),
        ]
    )
    assert code == 0
    assert calls == [("win", "v1.2.3-dev")]
    assert "fake-setup.exe" in capsys.readouterr().out


# freedesktop.org registered categories (menu-spec "Registered Categories") that the
# desktop entry may use; appimagetool's desktop-file-validate rejects anything else
# that does not start with "X-" (release run 44458756513 failed on "Synthesizer").
_REGISTERED_CATEGORIES = {"AudioVideo", "Audio", "Music", "Midi", "Player", "Recorder"}


def test_desktop_entry_categories_are_registered():
    line = next(x for x in ri.DESKTOP_ENTRY.splitlines() if x.startswith("Categories="))
    cats = [c for c in line.removeprefix("Categories=").split(";") if c]
    assert cats, "Categories must not be empty"
    assert set(cats) <= _REGISTERED_CATEGORIES, cats
