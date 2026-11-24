import plistlib
from pathlib import Path

import pytest
import release_installers as ri
import release_package as rp
from test_release_package import LIN, MAC, NOTICES_OK, WIN, make_tree


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
    assert r"{commoncf64}\VST3\nd-matrix-1.vst3" in iss
    assert r"{commoncf64}\CLAP" in iss
    assert "VersionInfoVersion=1.2.3" in iss
    assert "LicenseFile=" in iss
    assert "AppSupportURL" not in iss
    assert "THIRD_PARTY_LICENSES.html" in iss
    assert "ignoreversion" in next(
        line for line in iss.splitlines() if "THIRD_PARTY_LICENSES.html" in line
    )
    for name in ("ADDITIONAL-PERMISSIONS.md", "TRADEMARKS.md"):
        assert f"\\{name}" in iss

    stage_dir = work_dir / "win-inno-stage"
    assert (stage_dir / "ns4.exe").exists()
    assert (stage_dir / "nd-matrix-1.exe").exists()
    assert (stage_dir / "nd-matrix-1.clap").exists()
    assert (
        stage_dir / "nd-matrix-1.vst3" / "Contents" / "x86_64-win" / "nd-matrix-1.vst3"
    ).exists()
    assert not (stage_dir / "stray.ns4b").exists()
    assert (stage_dir / "THIRD_PARTY_LICENSES.html").exists()
    for name in ("ADDITIONAL-PERMISSIONS.md", "TRADEMARKS.md"):
        assert (stage_dir / name).exists()


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
    assert (pkg_root / "usr" / "bin" / "nd-matrix-1-standalone").exists()
    assert (pkg_root / "usr" / "lib" / "clap" / "nd-matrix-1.clap").exists()
    assert (
        pkg_root
        / "usr"
        / "lib"
        / "vst3"
        / "nd-matrix-1.vst3"
        / "Contents"
        / "x86_64-linux"
        / "nd-matrix-1.so"
    ).exists()
    assert (pkg_root / "usr" / "share" / "applications" / "nd-matrix-1.desktop").exists()
    doc_dir = pkg_root / "usr" / "share" / "doc" / "nd-matrix-1"
    assert (doc_dir / "LICENSE-PLUGIN").exists()
    assert (doc_dir / "LICENSE").exists()
    assert (doc_dir / "THIRD_PARTY_LICENSES.html").read_text(encoding="utf-8") == NOTICES_OK
    for name in ("ADDITIONAL-PERMISSIONS.md", "TRADEMARKS.md"):
        assert (doc_dir / name).exists()


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
    doc_dir = appdir / "usr" / "share" / "doc" / "nd-matrix-1"
    assert (doc_dir / "LICENSE").exists()
    assert (doc_dir / "THIRD_PARTY_LICENSES.html").exists()
    for name in ("ADDITIONAL-PERMISSIONS.md", "TRADEMARKS.md"):
        assert (doc_dir / name).exists()
    assert (appdir / "usr" / "lib" / "clap" / "nd-matrix-1.clap").exists()
    assert (
        appdir
        / "usr"
        / "lib"
        / "vst3"
        / "nd-matrix-1.vst3"
        / "Contents"
        / "x86_64-linux"
        / "nd-matrix-1.so"
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
    assert (dmg_stage / "nd-matrix-1.vst3" / "Contents" / "MacOS" / "nd-matrix-1").exists()
    assert (dmg_stage / "nd-matrix-1.clap" / "Contents" / "MacOS" / "nd-matrix-1").exists()
    assert (dmg_stage / "ns4").exists()
    assert (dmg_stage / "LICENSE-PLUGIN").exists()
    assert (dmg_stage / "THIRD_PARTY_LICENSES.html").exists()
    for name in ("ADDITIONAL-PERMISSIONS.md", "TRADEMARKS.md"):
        assert (dmg_stage / name).exists()


def test_stage_macos_pkg_roots_and_distribution_xml(tmp_path):
    root = _make_tree_with_licenses(tmp_path, MAC)
    paths = ri.stage_macos_pkg_roots("v1.2.3", tmp_path / "dist", tmp_path / "work", root=root)
    assert paths["pkg_path"].name == "nd-matrix-1-v1.2.3-macos-arm64.pkg"
    assert (
        paths["app_root"] / "Applications" / "ND Matrix 1.app" / "Contents" / "Info.plist"
    ).exists()
    assert (paths["app_root"] / "usr" / "local" / "bin" / "ns4").exists()
    app_doc = paths["app_root"] / "usr" / "local" / "share" / "doc" / "nd-matrix-1"
    assert (app_doc / "THIRD_PARTY_LICENSES.html").exists()
    for name in ("ADDITIONAL-PERMISSIONS.md", "TRADEMARKS.md"):
        assert (app_doc / name).exists()
    assert (app_doc / "LICENSE").exists()
    for name in ("ADDITIONAL-PERMISSIONS.md", "TRADEMARKS.md"):
        assert (paths["resources"] / name).exists()
    assert (paths["resources"] / "THIRD_PARTY_LICENSES.html").read_text(
        encoding="utf-8"
    ) == NOTICES_OK
    assert (
        paths["vst3_root"]
        / "Library"
        / "Audio"
        / "Plug-Ins"
        / "VST3"
        / "nd-matrix-1.vst3"
        / "Contents"
        / "MacOS"
        / "nd-matrix-1"
    ).exists()
    assert (
        paths["clap_root"]
        / "Library"
        / "Audio"
        / "Plug-Ins"
        / "CLAP"
        / "nd-matrix-1.clap"
        / "Contents"
        / "MacOS"
        / "nd-matrix-1"
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


def test_inno_script_removes_legacy_plugin_bundles(tmp_path):
    root = _make_tree_with_licenses(tmp_path, WIN)
    iss_path, _ = ri.stage_windows_inno("v1.2.3", tmp_path / "dist", tmp_path / "work", root=root)
    iss = iss_path.read_text(encoding="utf-8")
    assert "[InstallDelete]" in iss
    assert (
        r'Type: filesandordirs; Name: "{commoncf64}\VST3\nord-plugin.vst3"; Components: vst3' in iss
    )
    assert r'Type: files; Name: "{commoncf64}\CLAP\nord-plugin.clap"; Components: clap' in iss
    assert r"{commoncf64}\VST3\nd-matrix-1.vst3" in iss
    assert r"{commoncf64}\CLAP" in iss
    assert "nord-plugin.clap" not in iss.split("[Files]")[1].split("[InstallDelete]")[0]


def test_appimage_install_plugins_removes_legacy_and_installs_new_names(tmp_path):
    root = _make_tree_with_licenses(tmp_path, LIN)
    appdir, _ = ri.stage_linux_appdir("v1.2.3", tmp_path / "dist", tmp_path / "work", root=root)
    script = (appdir / "AppRun").read_text(encoding="utf-8")
    assert "@NEW@" not in script and "@LEG@" not in script
    assert 'rm -f "$HOME/.clap/nord-plugin.clap"' in script
    assert 'rm -rf "$HOME/.vst3/nord-plugin.vst3"' in script
    assert '"$HOME/.clap/nd-matrix-1.clap"' in script
    assert 'cp -R "$HERE/usr/lib/vst3/nd-matrix-1.vst3" "$HOME/.vst3/nd-matrix-1.vst3"' in script
    # The legacy files are removed before the new ones are copied in.
    assert script.index("nord-plugin.clap") < script.index('cp -f "$HERE/usr/lib/clap')


def test_apprun_install_plugins_replaces_legacy_files_on_disk(tmp_path):
    """Run the real `--install-plugins` branch with a fake HOME holding legacy bundles."""
    import subprocess

    root = _make_tree_with_licenses(tmp_path, LIN)
    appdir, _ = ri.stage_linux_appdir("v1.2.3", tmp_path / "dist", tmp_path / "work", root=root)
    home = tmp_path / "home"
    (home / ".clap").mkdir(parents=True)
    (home / ".vst3" / "nord-plugin.vst3" / "Contents").mkdir(parents=True)
    (home / ".clap" / "nord-plugin.clap").write_bytes(b"old")
    done = subprocess.run(
        ["sh", str(appdir / "AppRun"), "--install-plugins"],
        env={"HOME": str(home), "PATH": "/usr/bin:/bin"},
        capture_output=True,
        text=True,
        check=True,
    )
    assert "nd-matrix-1.clap" in done.stdout
    assert sorted(p.name for p in (home / ".clap").iterdir()) == ["nd-matrix-1.clap"]
    assert sorted(p.name for p in (home / ".vst3").iterdir()) == ["nd-matrix-1.vst3"]


def test_deb_ships_only_new_bundle_names(tmp_path):
    root = _make_tree_with_licenses(tmp_path, LIN)
    pkg_root, _ = ri.stage_linux_deb("v1.2.3", tmp_path / "dist", tmp_path / "work", root=root)
    assert [p.name for p in (pkg_root / "usr" / "lib" / "clap").iterdir()] == ["nd-matrix-1.clap"]
    assert [p.name for p in (pkg_root / "usr" / "lib" / "vst3").iterdir()] == ["nd-matrix-1.vst3"]


def test_macos_pkg_component_preinstall_scripts_remove_legacy_bundles(tmp_path):
    root = _make_tree_with_licenses(tmp_path, MAC)
    paths = ri.stage_macos_pkg_roots("v1.2.3", tmp_path / "dist", tmp_path / "work", root=root)
    vst3 = (paths["scripts_vst3"] / "preinstall").read_text(encoding="utf-8")
    clap = (paths["scripts_clap"] / "preinstall").read_text(encoding="utf-8")
    assert vst3.startswith("#!/bin/sh\n")
    assert 'rm -rf "/Library/Audio/Plug-Ins/VST3/nord-plugin.vst3"' in vst3
    assert 'rm -rf "/Library/Audio/Plug-Ins/CLAP/nord-plugin.clap"' in clap
    assert (paths["scripts_vst3"] / "preinstall").stat().st_mode & 0o111
    assert (paths["scripts_clap"] / "preinstall").stat().st_mode & 0o111
    xml = paths["dist_xml"].read_text(encoding="utf-8")
    assert "nd-matrix-1.vst3" in xml and "nd-matrix-1.clap" in xml
    assert "nord-plugin" not in xml


def test_build_macos_pkg_passes_scripts_to_plugin_components_only(tmp_path, monkeypatch):
    root = _make_tree_with_licenses(tmp_path, MAC)
    calls = []
    monkeypatch.setattr(ri.subprocess, "run", lambda cmd, **kw: calls.append(cmd))
    monkeypatch.setattr(ri, "write_sha256", lambda p: None)
    ri.build_macos_pkg("v1.2.3", tmp_path / "dist", tmp_path / "work", root=root)
    builds = {
        c[c.index("--identifier") + 1].rsplit(".", 1)[1]: c for c in calls if c[0] == "pkgbuild"
    }
    assert "--scripts" not in builds["app"]
    assert builds["vst3"][builds["vst3"].index("--scripts") + 1].endswith("scripts-vst3")
    assert builds["clap"][builds["clap"].index("--scripts") + 1].endswith("scripts-clap")


def test_dmg_stage_uses_new_bundle_names_only(tmp_path):
    root = _make_tree_with_licenses(tmp_path, MAC)
    dmg_stage, _ = ri.stage_macos_dmg(
        "v1.2.3", tmp_path / "dist", tmp_path / "work", root=root, create_symlinks=False
    )
    names = {p.name for p in dmg_stage.iterdir()}
    assert {"nd-matrix-1.vst3", "nd-matrix-1.clap"} <= names
    assert not {n for n in names if n.startswith("nord-plugin.")}


def test_appdir_ships_appimage_runtime_notices(tmp_path):
    root = _make_tree_with_licenses(tmp_path, LIN)
    appdir, _ = ri.stage_linux_appdir("v1.2.3", tmp_path / "dist", tmp_path / "work", root=root)
    notices = appdir / "usr" / "share" / "doc" / "nd-matrix-1" / "appimage-runtime"
    names = {p.name for p in notices.iterdir()}
    assert names == set(rp.APPIMAGE_RUNTIME_FILES)
    assert "LESSER" in (notices / "libfuse-LGPL-2.1.txt").read_text(encoding="utf-8")


def test_deb_does_not_ship_appimage_runtime_notices(tmp_path):
    root = _make_tree_with_licenses(tmp_path, LIN)
    pkg_root, _ = ri.stage_linux_deb("v1.2.3", tmp_path / "dist", tmp_path / "work", root=root)
    assert not list(pkg_root.rglob("appimage-runtime"))


def test_build_appimage_rejects_unpinned_runtime(tmp_path):
    root = _make_tree_with_licenses(tmp_path, LIN)
    args = ("v1.2.3", tmp_path / "dist", tmp_path / "work", "appimagetool", root)
    with pytest.raises(ValueError, match="--appimage-runtime is required"):
        ri.build_linux_appimage(*args)
    other = tmp_path / "runtime"
    other.write_bytes(b"not the pinned runtime")
    with pytest.raises(ValueError, match="expected the pinned"):
        ri.build_linux_appimage(*args, None, other)


def test_build_appimage_passes_pinned_runtime_to_appimagetool(tmp_path, monkeypatch):
    root = _make_tree_with_licenses(tmp_path, LIN)
    runtime = tmp_path / "runtime"
    runtime.write_bytes(b"runtime")
    monkeypatch.setattr(ri, "APPIMAGE_RUNTIME_SHA256", ri.sha256_hex(runtime))
    calls = []

    def fake_run(cmd, env, check):
        calls.append(cmd)
        Path(cmd[-1]).write_bytes(b"appimage")

    monkeypatch.setattr(ri.subprocess, "run", fake_run)
    out = ri.build_linux_appimage(
        "v1.2.3", tmp_path / "dist", tmp_path / "work", "tool", root, None, runtime
    )
    assert calls == [
        [
            "tool",
            "--runtime-file",
            str(runtime),
            str(tmp_path / "work" / "NdMatrix1.AppDir"),
            str(out),
        ]
    ]
    assert out.with_name(out.name + ".sha256").exists()


def test_main_requires_runtime_for_appimage(tmp_path):
    root = _make_tree_with_licenses(tmp_path, LIN)
    with pytest.raises(ValueError, match="--appimage-runtime is required"):
        ri.main(
            [
                "--target",
                LIN.triple,
                "--formats",
                "appimage",
                "--out",
                str(tmp_path / "dist"),
                "--work",
                str(tmp_path / "work"),
                "--root",
                str(root),
            ]
        )
