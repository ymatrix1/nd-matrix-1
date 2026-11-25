import pytest
import release_bundle_libs as bl


def _tree(root, sonames=None, with_copyright=True):
    lib_dir, doc_root = root / "lib", root / "doc"
    lib_dir.mkdir()
    for soname, package in (sonames or bl.BUNDLED_LIBS).items():
        (lib_dir / soname).write_bytes(f"lib {soname}".encode())
        (doc_root / package).mkdir(parents=True)
        if with_copyright:
            (doc_root / package / "copyright").write_text(f"(c) {package}", encoding="utf-8")
    return lib_dir, doc_root


def test_bundled_libs_cover_the_winit_xkb_libraries():
    assert "libxkbcommon.so.0" in bl.BUNDLED_LIBS
    assert "libxkbcommon-x11.so.0" in bl.BUNDLED_LIBS
    assert all(name.endswith(("so.0", "so.1")) for name in bl.BUNDLED_LIBS)


def test_find_lib_searches_directories_in_order(tmp_path):
    first, second = tmp_path / "a", tmp_path / "b"
    first.mkdir()
    second.mkdir()
    (second / "libx.so.0").write_bytes(b"x")
    assert bl.find_lib("libx.so.0", [first, second]) == second / "libx.so.0"
    (first / "libx.so.0").write_bytes(b"y")
    assert bl.find_lib("libx.so.0", [first, second]) == first / "libx.so.0"


def test_find_lib_missing_raises(tmp_path):
    with pytest.raises(FileNotFoundError, match="libnope.so.0 not found"):
        bl.find_lib("libnope.so.0", [tmp_path])


def test_bundle_libs_copies_libs_and_notices(tmp_path):
    lib_dir, doc_root = _tree(tmp_path)
    appdir = tmp_path / "AppDir"
    copied = bl.bundle_libs(appdir, [lib_dir], doc_root)
    assert [p.name for p in copied] == list(bl.BUNDLED_LIBS)
    for soname, package in bl.BUNDLED_LIBS.items():
        assert (appdir / "usr" / "lib" / soname).read_bytes() == f"lib {soname}".encode()
        notice = appdir / "usr/share/doc/nd-matrix-1/bundled-libs" / f"{package}.copyright"
        assert notice.read_text(encoding="utf-8") == f"(c) {package}"


def test_bundle_libs_missing_library_fails(tmp_path):
    lib_dir, doc_root = _tree(tmp_path)
    (lib_dir / "libxkbcommon-x11.so.0").unlink()
    with pytest.raises(FileNotFoundError, match="libxkbcommon-x11.so.0 not found"):
        bl.bundle_libs(tmp_path / "AppDir", [lib_dir], doc_root)


def test_bundle_libs_missing_copyright_fails(tmp_path):
    lib_dir, doc_root = _tree(tmp_path, with_copyright=False)
    with pytest.raises(FileNotFoundError, match="missing .*copyright"):
        bl.bundle_libs(tmp_path / "AppDir", [lib_dir], doc_root)


def test_bundle_libs_accepts_a_custom_set(tmp_path):
    only = {"libxkbcommon.so.0": "libxkbcommon0"}
    lib_dir, doc_root = _tree(tmp_path, only)
    copied = bl.bundle_libs(tmp_path / "AppDir", [lib_dir], doc_root, only)
    assert [p.name for p in copied] == ["libxkbcommon.so.0"]
