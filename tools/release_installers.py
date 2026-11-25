#!/usr/bin/env python3
"""Build native OS installers (Windows Inno Setup, macOS DMG/PKG, Linux DEB/AppImage).

Reuses the allowlisted build outputs from `tools/release_package.py` so no
forbidden files (`.reference/`, firmware dumps, presets) can enter any installer.
Each installer packages the standalone apps (`ns4`, `nd-matrix-1`,
`nd-matrix-1-standalone`) and installs (or lets the user select) the CLAP and
VST3 DAW plugins into the standard OS plugin folders.
"""

from __future__ import annotations

import argparse
import os
import plistlib
import re
import shutil
import subprocess
import sys
from collections.abc import Sequence
from pathlib import Path

from release_bundle_libs import DEFAULT_DOC_ROOT, DEFAULT_LIB_DIRS, bundle_libs
from release_package import (
    APPIMAGE_RUNTIME_DIR,
    APPIMAGE_RUNTIME_SHA256,
    BINS,
    DOC_FILES,
    LEGACY_PLUGIN_STEM,
    LEGACY_STANDALONE_BIN,
    LEGAL_NOTICES,
    PLATFORMS,
    PLUGIN_STEM,
    ROOT,
    STANDALONE_BIN,
    THIRD_PARTY_NOTICES,
    Platform,
    collect,
    sha256_hex,
    write_sha256,
)

APP_DISPLAY_NAME = "ND Matrix 1"
APP_BUNDLE_NAME = "ND Matrix 1.app"
APP_BUNDLE_ID = "org.ndmatrix.nd-matrix-1"
APP_URL = "https://github.com/ymatrix1/nd-matrix-1"
APP_PUBLISHER = "ND Matrix 1 Contributors"

DESKTOP_ENTRY = """[Desktop Entry]
Type=Application
Name=ND Matrix 1
Comment=Source-available DSP sound engine and front-panel keyboard emulator
Exec=nd-matrix-1
Icon=nd-matrix-1
Categories=AudioVideo;Audio;Music;Midi;
Terminal=false
"""

# Minimal monochrome SVG icon so .desktop and AppImage have a valid icon without external assets.
APP_SVG_ICON = """<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 128 128">
  <rect width="128" height="128" rx="16" fill="#b71c1c"/>
  <rect x="12" y="18" width="104" height="24" rx="4" fill="#1a1a1a"/>
  <text x="64" y="35" font-family="sans-serif" font-size="14" font-weight="bold"
        fill="#f5f5f5" text-anchor="middle">MATRIX 1</text>
  <rect x="12" y="54" width="104" height="56" rx="4" fill="#f5f5f5"/>
  <rect x="24" y="54" width="10" height="34" fill="#1a1a1a"/>
  <rect x="42" y="54" width="10" height="34" fill="#1a1a1a"/>
  <rect x="70" y="54" width="10" height="34" fill="#1a1a1a"/>
  <rect x="88" y="54" width="10" height="34" fill="#1a1a1a"/>
  <rect x="104" y="54" width="10" height="34" fill="#1a1a1a"/>
</svg>
"""

APPRUN_SCRIPT = """#!/bin/sh
set -eu
HERE="$(dirname "$(readlink -f "$0")")"
export PATH="$HERE/usr/bin:$PATH"
# Bundled xkbcommon libraries (winit loads them at start-up; D248).
export LD_LIBRARY_PATH="$HERE/usr/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"

case "${1:-}" in
  --cli)
    shift
    exec "$HERE/usr/bin/ns4" "$@"
    ;;
  --standalone-plugin)
    shift
    exec "$HERE/usr/bin/@STANDALONE@" "$@"
    ;;
  --install-plugins)
    mkdir -p "$HOME/.clap" "$HOME/.vst3"
    # Remove the pre-rename bundles so hosts do not list two copies.
    rm -f "$HOME/.clap/@LEG@.clap"
    rm -rf "$HOME/.vst3/@LEG@.vst3"
    cp -f "$HERE/usr/lib/clap/@NEW@.clap" "$HOME/.clap/@NEW@.clap"
    rm -rf "$HOME/.vst3/@NEW@.vst3"
    cp -R "$HERE/usr/lib/vst3/@NEW@.vst3" "$HOME/.vst3/@NEW@.vst3"
    echo "Installed CLAP to $HOME/.clap/@NEW@.clap"
    echo "Installed VST3 to $HOME/.vst3/@NEW@.vst3"
    exit 0
    ;;
esac

if [ "$(basename "$0")" = "ns4" ]; then
  exec "$HERE/usr/bin/ns4" "$@"
fi
exec "$HERE/usr/bin/nd-matrix-1" "$@"
"""


LICENSE_NOTICE = "Licensed under PolyForm Noncommercial 1.0.0: commercial use is not permitted."


def write_macos_license(raw_stage: Path, resources: Path) -> None:
    """Copy the staged LICENSE into the pkg resources, or write the notice if it is absent."""
    if (raw_stage / "LICENSE").exists():
        shutil.copy2(raw_stage / "LICENSE", resources / "LICENSE")
    else:
        (resources / "LICENSE").write_text(LICENSE_NOTICE + "\n", encoding="utf-8")


def render_apprun() -> str:
    """AppRun script with the current and legacy plugin bundle names filled in."""
    return (
        APPRUN_SCRIPT.replace("@NEW@", PLUGIN_STEM)
        .replace("@LEG@", LEGACY_PLUGIN_STEM)
        .replace("@STANDALONE@", STANDALONE_BIN)
    )


def render_macos_preinstall(kind: str, install_dir: str) -> str:
    """`pkgbuild --scripts` preinstall that deletes the legacy `kind` (`clap`/`vst3`) bundle."""
    return (
        "#!/bin/sh\n"
        "# Remove the pre-rename bundle so hosts do not list two copies.\n"
        f'rm -rf "{install_dir}/{LEGACY_PLUGIN_STEM}.{kind}"\n'
        "exit 0\n"
    )


def _copy_allowlisted(platform: Platform, root: Path, target_dir: Path | None, dest: Path) -> None:
    """Copy allowlisted release files into `dest` preserving relative layout."""
    for src, rel in collect(platform, root, target_dir):
        out = dest / rel
        out.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(src, out)


def _strip_v(label: str) -> str:
    return label[1:] if label.startswith("v") else label


def debian_version(label: str) -> str:
    """Convert a release label (`v0.1.0` or `v0.1.0-rc.1`) to a valid Debian version."""
    ver = _strip_v(label)
    if "-" in ver:
        base, pre = ver.split("-", 1)
        return f"{base}~{pre}"
    return ver


def numeric_version(label: str) -> str:
    """Extract `X.Y.Z` numeric version for Windows/macOS installer metadata."""
    match = re.match(r"^v?(\d+\.\d+\.\d+)", label)
    if not match:
        raise ValueError(f"cannot extract numeric X.Y.Z version from {label!r}")
    return match.group(1)


# ---------------------------------------------------------------------------
# Windows: Inno Setup (.exe)
# ---------------------------------------------------------------------------


def render_inno_iss(
    label: str,
    stage_dir: Path,
    out_dir: Path,
    output_base_filename: str,
) -> str:
    """Render an Inno Setup `.iss` script for standalone apps + selectable CLAP/VST3."""
    ver = numeric_version(label)
    src = str(stage_dir.resolve())
    out = str(out_dir.resolve())
    license_line = f"LicenseFile={src}\\LICENSE\n" if (stage_dir / "LICENSE").exists() else ""
    lines = [
        "; Auto-generated by tools/release_installers.py",
        "[Setup]",
        f"AppId={{{{{APP_BUNDLE_ID}}}}}",
        f"AppName={APP_DISPLAY_NAME}",
        f"AppVersion={_strip_v(label)}",
        f"AppVerName={APP_DISPLAY_NAME} {_strip_v(label)}",
        f"VersionInfoVersion={ver}",
        f"AppPublisher={APP_PUBLISHER}",
        f"AppPublisherURL={APP_URL}",
        f"DefaultDirName={{autopf}}\\{APP_DISPLAY_NAME}",
        f"DefaultGroupName={APP_DISPLAY_NAME}",
        "ArchitecturesAllowed=x64compatible",
        "ArchitecturesInstallIn64BitMode=x64compatible",
        "ChangesEnvironment=yes",
        "Compression=lzma2",
        "SolidCompression=yes",
        "WizardStyle=modern",
        f"InfoBeforeFile={src}\\README.txt",
        f"{license_line}OutputDir={out}",
        f"OutputBaseFilename={output_base_filename}",
        "UninstallDisplayIcon={app}\\nd-matrix-1.exe",
        "",
        "[Types]",
        'Name: "full"; Description: "Full installation (GUI, CLI, VST3 & CLAP plugins)"',
        'Name: "standalone"; Description: "Standalone GUI & CLI only"',
        'Name: "plugins"; Description: "DAW Plugins only (VST3 & CLAP)"',
        'Name: "custom"; Description: "Custom installation"; Flags: iscustom',
        "",
        "[Components]",
        (
            'Name: "app"; Description: "Standalone GUI (nd-matrix-1) & Plugin Host"; '
            "Types: full standalone custom"
        ),
        'Name: "cli"; Description: "Command-Line Tool (ns4)"; Types: full standalone custom',
        'Name: "vst3"; Description: "VST3 Plugin (Common Files\\VST3)"; Types: full plugins custom',
        'Name: "clap"; Description: "CLAP Plugin (Common Files\\CLAP)"; Types: full plugins custom',
        "",
        "[Tasks]",
        (
            'Name: "desktopicon"; Description: "Create a &desktop shortcut"; '
            'GroupDescription: "Additional shortcuts:"; Components: app; Flags: unchecked'
        ),
        (
            'Name: "addtopath"; Description: "Add install folder to user PATH (for ns4 CLI)"; '
            'GroupDescription: "Environment:"; Components: cli'
        ),
        "",
        "[Files]",
        (
            f'Source: "{src}\\nd-matrix-1.exe"; DestDir: "{{app}}"; '
            "Components: app; Flags: ignoreversion"
        ),
        (
            f'Source: "{src}\\{STANDALONE_BIN}.exe"; DestDir: "{{app}}"; '
            "Components: app; Flags: ignoreversion"
        ),
        f'Source: "{src}\\ns4.exe"; DestDir: "{{app}}"; Components: cli; Flags: ignoreversion',
        f'Source: "{src}\\README.txt"; DestDir: "{{app}}"; Flags: ignoreversion',
        f'Source: "{src}\\HOST_COMPATIBILITY.md"; DestDir: "{{app}}"; Flags: ignoreversion',
        f'Source: "{src}\\{THIRD_PARTY_NOTICES}"; DestDir: "{{app}}"; Flags: ignoreversion',
        *(
            f'Source: "{src}\\{name}"; DestDir: "{{app}}"; Flags: ignoreversion'
            for name in LEGAL_NOTICES
        ),
        (
            f'Source: "{src}\\LICENSE*"; DestDir: "{{app}}"; '
            "Flags: ignoreversion skipifsourcedoesntexist"
        ),
        (
            f'Source: "{src}\\{PLUGIN_STEM}.vst3\\*"; '
            f'DestDir: "{{commoncf64}}\\VST3\\{PLUGIN_STEM}.vst3"; '
            "Components: vst3; Flags: ignoreversion recursesubdirs createallsubdirs"
        ),
        (
            f'Source: "{src}\\{PLUGIN_STEM}.clap"; DestDir: "{{commoncf64}}\\CLAP"; '
            "Components: clap; Flags: ignoreversion"
        ),
        "",
        "[InstallDelete]",
        (
            f'Type: filesandordirs; Name: "{{commoncf64}}\\VST3\\{LEGACY_PLUGIN_STEM}.vst3"; '
            "Components: vst3"
        ),
        (f'Type: files; Name: "{{commoncf64}}\\CLAP\\{LEGACY_PLUGIN_STEM}.clap"; Components: clap'),
        f'Type: files; Name: "{{app}}\\{LEGACY_STANDALONE_BIN}.exe"; Components: app',
        "",
        "[Icons]",
        (
            f'Name: "{{group}}\\{APP_DISPLAY_NAME}"; Filename: "{{app}}\\nd-matrix-1.exe"; '
            "Components: app"
        ),
        f'Name: "{{group}}\\Uninstall {APP_DISPLAY_NAME}"; Filename: "{{uninstallexe}}"',
        (
            f'Name: "{{autodesktop}}\\{APP_DISPLAY_NAME}"; Filename: "{{app}}\\nd-matrix-1.exe"; '
            "Tasks: desktopicon"
        ),
        "",
        "[Registry]",
        (
            'Root: HKCU; Subkey: "Environment"; ValueType: expandsz; ValueName: "Path"; '
            'ValueData: "{olddata};{app}"; Tasks: addtopath; '
            "Check: NeedsAddPath(ExpandConstant('{app}'))"
        ),
        "",
        "[Code]",
        "function NeedsAddPath(Param: string): boolean;",
        "var",
        "  OrigPath: string;",
        "begin",
        "  if not RegQueryStringValue(HKEY_CURRENT_USER, 'Environment', 'Path', OrigPath) then",
        "  begin",
        "    Result := True;",
        "    exit;",
        "  end;",
        "  Result := Pos(';' + UpperCase(Param) + ';', ';' + UpperCase(OrigPath) + ';') = 0;",
        "end;",
        "",
    ]
    return "\n".join(lines)


def stage_windows_inno(
    label: str,
    out_dir: Path,
    work_dir: Path,
    root: Path = ROOT,
    target_dir: Path | None = None,
) -> tuple[Path, Path]:
    """Stage Windows binaries/plugins and write `.iss` script; return `(iss_path, exe_path)`."""
    platform = PLATFORMS["x86_64-pc-windows-msvc"]
    stage_dir = work_dir / "win-inno-stage"
    if stage_dir.exists():
        shutil.rmtree(stage_dir)
    _copy_allowlisted(platform, root, target_dir, stage_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    base_name = f"nd-matrix-1-{label}-{platform.name}-setup"
    iss_path = work_dir / f"{base_name}.iss"
    iss_path.write_text(
        render_inno_iss(label, stage_dir, out_dir, base_name),
        encoding="utf-8",
        newline="\r\n",
    )
    return iss_path, out_dir / f"{base_name}.exe"


def _find_iscc() -> str:
    found = shutil.which("iscc") or shutil.which("ISCC")
    if found:
        return found
    for candidate in (
        Path(r"C:\Program Files (x86)\Inno Setup 6\ISCC.exe"),
        Path(r"C:\Program Files\Inno Setup 6\ISCC.exe"),
    ):
        if candidate.exists():
            return str(candidate)
    raise FileNotFoundError("Inno Setup compiler (ISCC.exe) not found on PATH or in Program Files")


def build_windows_installer(
    label: str,
    out_dir: Path,
    work_dir: Path,
    root: Path = ROOT,
    target_dir: Path | None = None,
) -> Path:
    iss_path, exe_path = stage_windows_inno(label, out_dir, work_dir, root, target_dir)
    subprocess.run([_find_iscc(), str(iss_path)], check=True)
    write_sha256(exe_path)
    return exe_path


# ---------------------------------------------------------------------------
# Linux: .deb and .AppImage
# ---------------------------------------------------------------------------


def render_deb_control(label: str) -> str:
    return (
        "Package: nd-matrix-1\n"
        f"Version: {debian_version(label)}\n"
        "Section: sound\n"
        "Priority: optional\n"
        "Architecture: amd64\n"
        f"Maintainer: {APP_PUBLISHER} <noreply@github.com>\n"
        "Depends: libc6 (>= 2.38), libasound2t64 | libasound2, libgl1, libx11-6\n"
        f"Homepage: {APP_URL}\n"
        "Description: Source-available keyboard emulator: DSP engine, CLI, GUI, CLAP/VST3\n"
        f" Includes the ns4 CLI, nd-matrix-1 standalone GUI, {STANDALONE_BIN},\n"
        " and CLAP (/usr/lib/clap) + VST3 (/usr/lib/vst3) DAW plugins.\n"
    )


def stage_linux_deb(
    label: str,
    out_dir: Path,
    work_dir: Path,
    root: Path = ROOT,
    target_dir: Path | None = None,
) -> tuple[Path, Path]:
    """Create a Debian package staging directory; return `(pkg_root, deb_path)`."""
    platform = PLATFORMS["x86_64-unknown-linux-gnu"]
    raw_stage = work_dir / "linux-raw"
    if raw_stage.exists():
        shutil.rmtree(raw_stage)
    _copy_allowlisted(platform, root, target_dir, raw_stage)

    pkg_root = work_dir / "deb-root"
    if pkg_root.exists():
        shutil.rmtree(pkg_root)
    (pkg_root / "DEBIAN").mkdir(parents=True)
    (pkg_root / "usr" / "bin").mkdir(parents=True)
    (pkg_root / "usr" / "lib" / "clap").mkdir(parents=True)
    (pkg_root / "usr" / "lib" / "vst3").mkdir(parents=True)
    (pkg_root / "usr" / "share" / "applications").mkdir(parents=True)
    (pkg_root / "usr" / "share" / "icons" / "hicolor" / "scalable" / "apps").mkdir(parents=True)
    doc_dir = pkg_root / "usr" / "share" / "doc" / "nd-matrix-1"
    doc_dir.mkdir(parents=True)

    for b in BINS:
        dst = pkg_root / "usr" / "bin" / b
        shutil.copy2(raw_stage / b, dst)
        dst.chmod(0o755)

    clap_dst = pkg_root / "usr" / "lib" / "clap" / f"{PLUGIN_STEM}.clap"
    shutil.copy2(raw_stage / f"{PLUGIN_STEM}.clap", clap_dst)
    clap_dst.chmod(0o755)

    vst3_dst = pkg_root / "usr" / "lib" / "vst3" / f"{PLUGIN_STEM}.vst3"
    shutil.copytree(raw_stage / f"{PLUGIN_STEM}.vst3", vst3_dst)
    (vst3_dst / platform.vst3_binary).chmod(0o755)

    (pkg_root / "usr" / "share" / "applications" / "nd-matrix-1.desktop").write_text(
        DESKTOP_ENTRY, encoding="utf-8", newline="\n"
    )
    (
        pkg_root / "usr" / "share" / "icons" / "hicolor" / "scalable" / "apps" / "nd-matrix-1.svg"
    ).write_text(APP_SVG_ICON, encoding="utf-8", newline="\n")

    for doc in DOC_FILES:
        shutil.copy2(raw_stage / doc, doc_dir / doc)
    for lic in raw_stage.glob("LICENSE*"):
        shutil.copy2(lic, doc_dir / lic.name)

    control = pkg_root / "DEBIAN" / "control"
    control.write_text(render_deb_control(label), encoding="utf-8", newline="\n")

    out_dir.mkdir(parents=True, exist_ok=True)
    deb_path = out_dir / f"nd-matrix-1_{debian_version(label)}_amd64.deb"
    return pkg_root, deb_path


def build_linux_deb(
    label: str,
    out_dir: Path,
    work_dir: Path,
    root: Path = ROOT,
    target_dir: Path | None = None,
) -> Path:
    pkg_root, deb_path = stage_linux_deb(label, out_dir, work_dir, root, target_dir)
    subprocess.run(
        ["dpkg-deb", "--build", "--root-owner-group", str(pkg_root), str(deb_path)],
        check=True,
    )
    write_sha256(deb_path)
    return deb_path


def stage_linux_appdir(
    label: str,
    out_dir: Path,
    work_dir: Path,
    root: Path = ROOT,
    target_dir: Path | None = None,
    lib_dirs: Sequence[Path] | None = None,
    doc_root: Path = DEFAULT_DOC_ROOT,
) -> tuple[Path, Path]:
    """Create a Linux AppDir for `appimagetool`; return `(appdir, appimage_path)`.

    With `lib_dirs` the shared libraries of `release_bundle_libs.BUNDLED_LIBS` and their
    copyright files are copied in (D248); `None` skips that, for staging tests.
    """
    platform = PLATFORMS["x86_64-unknown-linux-gnu"]
    raw_stage = work_dir / "linux-raw-appimage"
    if raw_stage.exists():
        shutil.rmtree(raw_stage)
    _copy_allowlisted(platform, root, target_dir, raw_stage)

    appdir = work_dir / "NdMatrix1.AppDir"
    if appdir.exists():
        shutil.rmtree(appdir)
    (appdir / "usr" / "bin").mkdir(parents=True)
    (appdir / "usr" / "lib" / "clap").mkdir(parents=True)
    (appdir / "usr" / "lib" / "vst3").mkdir(parents=True)
    doc_dir = appdir / "usr" / "share" / "doc" / "nd-matrix-1"
    doc_dir.mkdir(parents=True)

    for b in BINS:
        dst = appdir / "usr" / "bin" / b
        shutil.copy2(raw_stage / b, dst)
        dst.chmod(0o755)

    clap_dst = appdir / "usr" / "lib" / "clap" / f"{PLUGIN_STEM}.clap"
    shutil.copy2(raw_stage / f"{PLUGIN_STEM}.clap", clap_dst)
    clap_dst.chmod(0o755)

    vst3_dst = appdir / "usr" / "lib" / "vst3" / f"{PLUGIN_STEM}.vst3"
    shutil.copytree(raw_stage / f"{PLUGIN_STEM}.vst3", vst3_dst)
    (vst3_dst / platform.vst3_binary).chmod(0o755)

    for doc in DOC_FILES:
        shutil.copy2(raw_stage / doc, doc_dir / doc)
    for lic in raw_stage.glob("LICENSE*"):
        shutil.copy2(lic, doc_dir / lic.name)
    # Notices of the runtime that appimagetool prepends (musl, LGPL libfuse, ...; D242).
    shutil.copytree(raw_stage / APPIMAGE_RUNTIME_DIR, doc_dir / "appimage-runtime")

    if lib_dirs is not None:
        bundle_libs(appdir, lib_dirs, doc_root)

    (appdir / "nd-matrix-1.desktop").write_text(DESKTOP_ENTRY, encoding="utf-8", newline="\n")
    (appdir / "nd-matrix-1.svg").write_text(APP_SVG_ICON, encoding="utf-8", newline="\n")
    apprun = appdir / "AppRun"
    apprun.write_text(render_apprun(), encoding="utf-8", newline="\n")
    apprun.chmod(0o755)

    out_dir.mkdir(parents=True, exist_ok=True)
    # Catalog naming (appimage.github.io): no "linux", no leading "v": `<name>-<version>-<arch>`.
    arch = platform.name.removeprefix("linux-")
    appimage_path = out_dir / f"nd-matrix-1-{_strip_v(label)}-{arch}.AppImage"
    return appdir, appimage_path


def check_appimage_runtime(runtime_file: Path | None) -> Path:
    """Return the runtime file after checking it is the pinned one (D242).

    Without `--runtime-file`, `appimagetool` downloads the moving `continuous` runtime, whose
    licenses we cannot vouch for; a different file would not match `licenses/appimage-runtime/`.
    """
    if runtime_file is None:
        raise ValueError("--appimage-runtime is required (pinned runtime, D242)")
    actual = sha256_hex(runtime_file)
    if actual != APPIMAGE_RUNTIME_SHA256:
        raise ValueError(
            f"{runtime_file} has sha256 {actual}, expected the pinned {APPIMAGE_RUNTIME_SHA256}"
        )
    return runtime_file


def build_linux_appimage(
    label: str,
    out_dir: Path,
    work_dir: Path,
    appimagetool: str = "appimagetool",
    root: Path = ROOT,
    target_dir: Path | None = None,
    runtime_file: Path | None = None,
    lib_dirs: Sequence[Path] = DEFAULT_LIB_DIRS,
    doc_root: Path = DEFAULT_DOC_ROOT,
) -> Path:
    runtime_file = check_appimage_runtime(runtime_file)
    appdir, appimage_path = stage_linux_appdir(
        label, out_dir, work_dir, root, target_dir, lib_dirs, doc_root
    )
    merged_env = {**os.environ, "ARCH": "x86_64", "APPIMAGE_EXTRACT_AND_RUN": "1"}
    cmd = [appimagetool, "--runtime-file", str(runtime_file), str(appdir), str(appimage_path)]
    subprocess.run(cmd, env=merged_env, check=True)
    write_sha256(appimage_path)
    return appimage_path


# ---------------------------------------------------------------------------
# macOS: .app bundle, .dmg, and .pkg
# ---------------------------------------------------------------------------


def create_macos_app_bundle(
    label: str,
    gui_binary: Path,
    dest_app: Path,
) -> Path:
    """Create `ND Matrix 1.app` wrapping `nd-matrix-1`."""
    if dest_app.exists():
        shutil.rmtree(dest_app)
    macos_dir = dest_app / "Contents" / "MacOS"
    macos_dir.mkdir(parents=True)
    bin_dst = macos_dir / "nd-matrix-1"
    shutil.copy2(gui_binary, bin_dst)
    bin_dst.chmod(0o755)
    plist = {
        "CFBundleDevelopmentRegion": "en",
        "CFBundleDisplayName": APP_DISPLAY_NAME,
        "CFBundleExecutable": "nd-matrix-1",
        "CFBundleIdentifier": f"{APP_BUNDLE_ID}.app",
        "CFBundleInfoDictionaryVersion": "6.0",
        "CFBundleName": "ND Matrix 1",
        "CFBundlePackageType": "APPL",
        "CFBundleShortVersionString": _strip_v(label),
        "CFBundleVersion": numeric_version(label),
        "LSMinimumSystemVersion": "11.0",
        "NSHighResolutionCapable": True,
    }
    with (dest_app / "Contents" / "Info.plist").open("wb") as handle:
        plistlib.dump(plist, handle)
    return dest_app


def stage_macos_dmg(
    label: str,
    out_dir: Path,
    work_dir: Path,
    root: Path = ROOT,
    target_dir: Path | None = None,
    *,
    create_symlinks: bool = True,
) -> tuple[Path, Path]:
    """Stage macOS `.dmg` folder with `.app`, plugins, CLI, and OS install symlinks."""
    platform = PLATFORMS["aarch64-apple-darwin"]
    raw_stage = work_dir / "macos-raw"
    if raw_stage.exists():
        shutil.rmtree(raw_stage)
    _copy_allowlisted(platform, root, target_dir, raw_stage)

    dmg_stage = work_dir / "macos-dmg-stage"
    if dmg_stage.exists():
        shutil.rmtree(dmg_stage)
    dmg_stage.mkdir(parents=True)

    create_macos_app_bundle(label, raw_stage / "nd-matrix-1", dmg_stage / APP_BUNDLE_NAME)
    shutil.copytree(raw_stage / f"{PLUGIN_STEM}.vst3", dmg_stage / f"{PLUGIN_STEM}.vst3")
    shutil.copytree(raw_stage / f"{PLUGIN_STEM}.clap", dmg_stage / f"{PLUGIN_STEM}.clap")
    for b in ("ns4", STANDALONE_BIN):
        dst = dmg_stage / b
        shutil.copy2(raw_stage / b, dst)
        dst.chmod(0o755)
    for doc in DOC_FILES:
        shutil.copy2(raw_stage / doc, dmg_stage / doc)
    for lic in raw_stage.glob("LICENSE*"):
        shutil.copy2(lic, dmg_stage / lic.name)

    if create_symlinks:
        (dmg_stage / "Applications").symlink_to("/Applications")
        (dmg_stage / "VST3 Plug-Ins").symlink_to("/Library/Audio/Plug-Ins/VST3")
        (dmg_stage / "CLAP Plug-Ins").symlink_to("/Library/Audio/Plug-Ins/CLAP")

    out_dir.mkdir(parents=True, exist_ok=True)
    dmg_path = out_dir / f"nd-matrix-1-{label}-{platform.name}.dmg"
    return dmg_stage, dmg_path


def render_macos_distribution_xml(label: str) -> str:
    ver = numeric_version(label)
    return f"""<?xml version="1.0" encoding="utf-8"?>
<installer-gui-script minSpecVersion="2">
    <title>{APP_DISPLAY_NAME} {_strip_v(label)}</title>
    <options customize="allow" require-scripts="false" hostArchitectures="arm64"/>
    <domains enable_localSystem="true"/>
    <readme file="README.txt"/>
    <license file="LICENSE"/>
    <choices-outline>
        <line choice="choice-app"/>
        <line choice="choice-vst3"/>
        <line choice="choice-clap"/>
    </choices-outline>
    <choice id="choice-app" title="Standalone GUI &amp; CLI (nd-matrix-1, ns4)"
            description="Installs ND Matrix 1.app to /Applications and ns4 to /usr/local/bin.">
        <pkg-ref id="{APP_BUNDLE_ID}.pkg.app"/>
    </choice>
    <choice id="choice-vst3" title="VST3 Plugin (/Library/Audio/Plug-Ins/VST3)"
            description="Installs nd-matrix-1.vst3 for DAWs such as Ableton Live, Reaper, Cubase.">
        <pkg-ref id="{APP_BUNDLE_ID}.pkg.vst3"/>
    </choice>
    <choice id="choice-clap" title="CLAP Plugin (/Library/Audio/Plug-Ins/CLAP)"
            description="Installs nd-matrix-1.clap for DAWs such as Bitwig Studio and Reaper.">
        <pkg-ref id="{APP_BUNDLE_ID}.pkg.clap"/>
    </choice>
    <pkg-ref id="{APP_BUNDLE_ID}.pkg.app" version="{ver}" onConclusion="none">app.pkg</pkg-ref>
    <pkg-ref id="{APP_BUNDLE_ID}.pkg.vst3" version="{ver}" onConclusion="none">vst3.pkg</pkg-ref>
    <pkg-ref id="{APP_BUNDLE_ID}.pkg.clap" version="{ver}" onConclusion="none">clap.pkg</pkg-ref>
</installer-gui-script>
"""


def stage_macos_pkg_roots(
    label: str,
    out_dir: Path,
    work_dir: Path,
    root: Path = ROOT,
    target_dir: Path | None = None,
) -> dict[str, Path]:
    """Stage component roots and `distribution.xml` for `pkgbuild`/`productbuild`."""
    platform = PLATFORMS["aarch64-apple-darwin"]
    raw_stage = work_dir / "macos-raw-pkg"
    if raw_stage.exists():
        shutil.rmtree(raw_stage)
    _copy_allowlisted(platform, root, target_dir, raw_stage)

    pkg_work = work_dir / "macos-pkg-work"
    if pkg_work.exists():
        shutil.rmtree(pkg_work)

    app_root = pkg_work / "root-app"
    (app_root / "Applications").mkdir(parents=True)
    (app_root / "usr" / "local" / "bin").mkdir(parents=True)
    doc_dir = app_root / "usr" / "local" / "share" / "doc" / "nd-matrix-1"
    doc_dir.mkdir(parents=True)
    create_macos_app_bundle(
        label, raw_stage / "nd-matrix-1", app_root / "Applications" / APP_BUNDLE_NAME
    )
    for b in ("ns4", STANDALONE_BIN):
        dst = app_root / "usr" / "local" / "bin" / b
        shutil.copy2(raw_stage / b, dst)
        dst.chmod(0o755)
    for doc in DOC_FILES:
        shutil.copy2(raw_stage / doc, doc_dir / doc)
    for lic in raw_stage.glob("LICENSE*"):
        shutil.copy2(lic, doc_dir / lic.name)

    vst3_root = pkg_work / "root-vst3"
    (vst3_root / "Library" / "Audio" / "Plug-Ins" / "VST3").mkdir(parents=True)
    shutil.copytree(
        raw_stage / f"{PLUGIN_STEM}.vst3",
        vst3_root / "Library" / "Audio" / "Plug-Ins" / "VST3" / f"{PLUGIN_STEM}.vst3",
    )

    clap_root = pkg_work / "root-clap"
    (clap_root / "Library" / "Audio" / "Plug-Ins" / "CLAP").mkdir(parents=True)
    shutil.copytree(
        raw_stage / f"{PLUGIN_STEM}.clap",
        clap_root / "Library" / "Audio" / "Plug-Ins" / "CLAP" / f"{PLUGIN_STEM}.clap",
    )

    for kind, key, folder in (("vst3", "vst3_root", "VST3"), ("clap", "clap_root", "CLAP")):
        script = pkg_work / f"scripts-{kind}" / "preinstall"
        script.parent.mkdir(parents=True)
        script.write_text(
            render_macos_preinstall(kind, f"/Library/Audio/Plug-Ins/{folder}"),
            encoding="utf-8",
            newline="\n",
        )
        script.chmod(0o755)

    resources = pkg_work / "resources"
    resources.mkdir(parents=True)
    shutil.copy2(raw_stage / "README.txt", resources / "README.txt")
    shutil.copy2(raw_stage / THIRD_PARTY_NOTICES, resources / THIRD_PARTY_NOTICES)
    for name in LEGAL_NOTICES:
        shutil.copy2(raw_stage / name, resources / name)
    write_macos_license(raw_stage, resources)

    dist_xml = pkg_work / "distribution.xml"
    dist_xml.write_text(render_macos_distribution_xml(label), encoding="utf-8", newline="\n")

    out_dir.mkdir(parents=True, exist_ok=True)
    pkg_path = out_dir / f"nd-matrix-1-{label}-{platform.name}.pkg"
    return {
        "pkg_work": pkg_work,
        "app_root": app_root,
        "vst3_root": vst3_root,
        "clap_root": clap_root,
        "scripts_vst3": pkg_work / "scripts-vst3",
        "scripts_clap": pkg_work / "scripts-clap",
        "resources": resources,
        "dist_xml": dist_xml,
        "pkg_path": pkg_path,
    }


def build_macos_dmg(
    label: str,
    out_dir: Path,
    work_dir: Path,
    root: Path = ROOT,
    target_dir: Path | None = None,
) -> Path:
    dmg_stage, dmg_path = stage_macos_dmg(label, out_dir, work_dir, root, target_dir)
    subprocess.run(
        [
            "hdiutil",
            "create",
            "-volname",
            f"ND Matrix 1 {_strip_v(label)}",
            "-srcfolder",
            str(dmg_stage),
            "-ov",
            "-format",
            "UDZO",
            str(dmg_path),
        ],
        check=True,
    )
    write_sha256(dmg_path)
    return dmg_path


def build_macos_pkg(
    label: str,
    out_dir: Path,
    work_dir: Path,
    root: Path = ROOT,
    target_dir: Path | None = None,
) -> Path:
    paths = stage_macos_pkg_roots(label, out_dir, work_dir, root, target_dir)
    ver = numeric_version(label)
    pkg_work = paths["pkg_work"]
    for name, root_key, id_suffix, scripts_key in (
        ("app.pkg", "app_root", "app", None),
        ("vst3.pkg", "vst3_root", "vst3", "scripts_vst3"),
        ("clap.pkg", "clap_root", "clap", "scripts_clap"),
    ):
        scripts_args = ["--scripts", str(paths[scripts_key])] if scripts_key else []
        subprocess.run(
            [
                "pkgbuild",
                "--root",
                str(paths[root_key]),
                *scripts_args,
                "--identifier",
                f"{APP_BUNDLE_ID}.pkg.{id_suffix}",
                "--version",
                ver,
                "--install-location",
                "/",
                str(pkg_work / name),
            ],
            check=True,
        )
    subprocess.run(
        [
            "productbuild",
            "--distribution",
            str(paths["dist_xml"]),
            "--resources",
            str(paths["resources"]),
            "--package-path",
            str(pkg_work),
            str(paths["pkg_path"]),
        ],
        check=True,
    )
    write_sha256(paths["pkg_path"])
    return paths["pkg_path"]


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--target", required=True, choices=sorted(PLATFORMS))
    parser.add_argument("--tag", help="release tag vX.Y.Z; omit for v<cargo_version>-dev")
    parser.add_argument("--out", type=Path, default=Path("dist"))
    parser.add_argument("--work", type=Path, default=Path("dist/.work"))
    parser.add_argument("--root", type=Path, default=ROOT)
    parser.add_argument("--target-dir", type=Path, help="cargo target dir (default <root>/target)")
    parser.add_argument(
        "--formats",
        nargs="+",
        choices=("inno", "deb", "appimage", "dmg", "pkg"),
        help="installer formats to build (default: all native formats for --target)",
    )
    parser.add_argument("--appimagetool", default="appimagetool")
    parser.add_argument(
        "--appimage-runtime",
        type=Path,
        help="pinned type2 runtime file (APPIMAGE_RUNTIME_URL); required for the appimage format",
    )
    args = parser.parse_args(argv)

    from release_package import version_from_tag, workspace_version

    cargo_version = workspace_version(args.root)
    label = f"v{version_from_tag(args.tag, cargo_version)}" if args.tag else f"v{cargo_version}-dev"
    default_formats = {
        "x86_64-pc-windows-msvc": ("inno",),
        "x86_64-unknown-linux-gnu": ("deb", "appimage"),
        "aarch64-apple-darwin": ("dmg", "pkg"),
    }[args.target]
    formats = args.formats or list(default_formats)

    args.work.mkdir(parents=True, exist_ok=True)
    built: list[Path] = []
    for fmt in formats:
        if fmt == "inno":
            built.append(
                build_windows_installer(label, args.out, args.work, args.root, args.target_dir)
            )
        elif fmt == "deb":
            built.append(build_linux_deb(label, args.out, args.work, args.root, args.target_dir))
        elif fmt == "appimage":
            built.append(
                build_linux_appimage(
                    label,
                    args.out,
                    args.work,
                    args.appimagetool,
                    args.root,
                    args.target_dir,
                    args.appimage_runtime,
                )
            )
        elif fmt == "dmg":
            built.append(build_macos_dmg(label, args.out, args.work, args.root, args.target_dir))
        elif fmt == "pkg":
            built.append(build_macos_pkg(label, args.out, args.work, args.root, args.target_dir))
    for path in built:
        print(path)
    return 0


if __name__ == "__main__":
    sys.exit(main())
