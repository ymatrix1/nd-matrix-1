#!/usr/bin/env python3
"""Bundle the few shared libraries the Linux AppImage cannot rely on the host to have (D248).

`winit` (egui) loads `libxkbcommon.so.0` and `libxkbcommon-x11.so.0` with `dlopen` at start-up.
A minimal host lacks them and the app exits before it shows a window (the AppImage catalog test
reported exactly this on v0.1.4). The release job installs the Debian packages, and this module
copies each library and the package's copyright file into the AppDir, so that the notice
requirement of the MIT-style licenses is met next to the binary.
"""

from __future__ import annotations

import shutil
from collections.abc import Sequence
from pathlib import Path

# ASSUMED: libxkbcommon-x11 pulls libxcb-xkb; libxcb, libX11 and libxkbcommon's own deps are
# present on any host that runs an X11 or Wayland session. Confirm by running the AppImage on a
# bare `ubuntu:22.04` container with only Xvfb (docs/RELEASING.md, "AppImage on a bare host").
BUNDLED_LIBS: dict[str, str] = {
    "libxkbcommon.so.0": "libxkbcommon0",
    "libxkbcommon-x11.so.0": "libxkbcommon-x11-0",
    "libxcb-xkb.so.1": "libxcb-xkb1",
}

# Debian/Ubuntu multiarch library directories on the release runner.
DEFAULT_LIB_DIRS: tuple[Path, ...] = (
    Path("/usr/lib/x86_64-linux-gnu"),
    Path("/lib/x86_64-linux-gnu"),
)
DEFAULT_DOC_ROOT = Path("/usr/share/doc")

# Where the libraries and their notices land inside the AppDir.
LIB_SUBDIR = ("usr", "lib")
NOTICE_SUBDIR = ("usr", "share", "doc", "nd-matrix-1", "bundled-libs")


def find_lib(soname: str, lib_dirs: Sequence[Path]) -> Path:
    """Return the first existing `soname` in `lib_dirs`, else raise `FileNotFoundError`."""
    for directory in lib_dirs:
        candidate = directory / soname
        if candidate.exists():
            return candidate
    raise FileNotFoundError(f"{soname} not found in {', '.join(str(d) for d in lib_dirs)}")


def bundle_libs(
    appdir: Path,
    lib_dirs: Sequence[Path] = DEFAULT_LIB_DIRS,
    doc_root: Path = DEFAULT_DOC_ROOT,
    libs: dict[str, str] | None = None,
) -> list[Path]:
    """Copy each library (named by its soname) and its package copyright into `appdir`.

    Symlinks are resolved so the copy is a regular file called `soname`. A missing library or a
    missing copyright file raises: shipping a binary without its notice is not allowed.
    """
    libs = BUNDLED_LIBS if libs is None else libs
    lib_out = appdir.joinpath(*LIB_SUBDIR)
    notice_out = appdir.joinpath(*NOTICE_SUBDIR)
    lib_out.mkdir(parents=True, exist_ok=True)
    notice_out.mkdir(parents=True, exist_ok=True)
    copied: list[Path] = []
    for soname, package in libs.items():
        src = find_lib(soname, lib_dirs).resolve()
        copyright_file = doc_root / package / "copyright"
        if not copyright_file.is_file():
            raise FileNotFoundError(f"missing {copyright_file} (notice of bundled {soname})")
        dst = lib_out / soname
        shutil.copy2(src, dst)
        dst.chmod(0o755)
        shutil.copy2(copyright_file, notice_out / f"{package}.copyright")
        copied.append(dst)
    return copied
