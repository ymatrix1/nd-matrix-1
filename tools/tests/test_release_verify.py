import struct

import pytest
import release_package as rp
import release_runtime_sources as rrs
import release_verify as rv
from test_release_package import LIN, MAC, NOTICES_OK, WIN, make_tree

README_OK = "This plugin has NOT been verified in any DAW yet."


def fake_pe(names):
    """Minimal PE32+ DLL image whose export table lists `names`."""
    raw, vaddr = 0x400, 0x1000
    image = bytearray(raw + 0x400)
    image[0:2] = b"MZ"
    struct.pack_into("<I", image, 0x3C, 0x40)
    image[0x40:0x44] = b"PE\0\0"
    struct.pack_into("<H", image, 0x40 + 6, 1)  # sections
    struct.pack_into("<H", image, 0x40 + 20, 240)  # optional header size
    opt = 0x40 + 24
    struct.pack_into("<H", image, opt, 0x20B)
    struct.pack_into("<I", image, opt + 112, vaddr)  # export directory RVA
    sec = opt + 240
    struct.pack_into("<IIII", image, sec + 8, 0x400, vaddr, 0x400, raw)
    exp = raw
    struct.pack_into("<I", image, exp + 24, len(names))
    struct.pack_into("<I", image, exp + 32, vaddr + 0x100)  # names array RVA
    text = 0x200
    for i, name in enumerate(names):
        struct.pack_into("<I", image, raw + 0x100 + 4 * i, vaddr + text)
        image[raw + text : raw + text + len(name) + 1] = name.encode() + b"\0"
        text += len(name) + 1
    return bytes(image)


def write_sources_asset(archive, skip=()):
    """Write the `-appimage-runtime-sources.tar.gz` next to a Linux archive."""
    import io
    import tarfile

    asset = archive.with_name(rrs.sources_name_for_archive(archive.name))
    with tarfile.open(asset, "w:gz") as tf:
        for name in rrs.SOURCES_FILES:
            if name in skip:
                continue
            info = tarfile.TarInfo(f"{asset.name[:-7]}/{name}")
            info.size = 4
            tf.addfile(info, io.BytesIO(b"data"))
    rp.write_sha256(asset)
    return asset


def build(tmp_path, platform, *, clap=None, vst3=None, readme=README_OK, extra=None):
    root = make_tree(tmp_path, platform)
    (root / "docs" / "RELEASE_README.txt").write_text(readme)
    bundled = root / "target" / "bundled"
    if platform.os == "macos":
        clap_path = bundled / "nd-matrix-1.clap" / "Contents" / "MacOS" / "nd-matrix-1"
        for plist in ("nd-matrix-1.clap", "nd-matrix-1.vst3"):
            (bundled / plist / "Contents" / "Info.plist").write_text("<plist/>")
    else:
        clap_path = bundled / "nd-matrix-1.clap"
    clap_path.write_bytes(clap if clap is not None else b"\0clap_entry\0")
    (bundled / "nd-matrix-1.vst3" / platform.vst3_binary).write_bytes(
        vst3 if vst3 is not None else b"\0GetPluginFactory\0"
    )
    if extra:
        for rel, data in extra.items():
            (root / rel).parent.mkdir(parents=True, exist_ok=True)
            (root / rel).write_bytes(data)
    archive = rp.package(platform.triple, "v1.2.3", tmp_path / "out", root)
    if platform.os == "linux":
        write_sources_asset(archive)
    return archive


def _drop(archive, member):
    """Rewrite `archive` (zip or tar.gz) without the file whose name ends in `member`."""
    import tarfile
    import zipfile

    files = rv.read_archive(archive)
    keep = {n: d for n, d in files.items() if not n.endswith(member)}
    assert len(keep) == len(files) - 1
    archive.unlink()
    if archive.name.endswith(".zip"):
        with zipfile.ZipFile(archive, "w") as zf:
            for n, d in keep.items():
                zf.writestr(n, d)
    else:
        import io

        with tarfile.open(archive, "w:gz") as tf:
            for n, d in keep.items():
                info = tarfile.TarInfo(n)
                info.size = len(d)
                tf.addfile(info, io.BytesIO(d))
    rp.write_sha256(archive)


@pytest.mark.parametrize("platform", [WIN, LIN, MAC])
def test_missing_third_party_notices_fails_release(tmp_path, platform):
    archive = build(tmp_path, platform)
    assert rv.verify(archive, platform.triple) == []
    _drop(archive, "/THIRD_PARTY_LICENSES.html")
    problems = rv.verify(archive, platform.triple)
    assert "missing THIRD_PARTY_LICENSES.html" in problems


@pytest.mark.parametrize("platform", [WIN, LIN, MAC])
@pytest.mark.parametrize("name", ["ADDITIONAL-PERMISSIONS.md", "TRADEMARKS.md"])
def test_missing_legal_notice_fails_release(tmp_path, platform, name):
    archive = build(tmp_path, platform)
    assert rv.verify(archive, platform.triple) == []
    _drop(archive, "/" + name)
    assert f"missing {name}" in rv.verify(archive, platform.triple)


def test_stub_or_wrong_third_party_notices_fail_release(tmp_path):
    stub = build(tmp_path / "a", LIN, extra={"THIRD_PARTY_LICENSES.html": b"<html></html>"})
    assert any("bytes (expected a full listing)" in p for p in rv.verify(stub, LIN.triple))
    no_text = b"x" * 5000
    bad = build(tmp_path / "b", LIN, extra={"THIRD_PARTY_LICENSES.html": no_text})
    problems = rv.verify(bad, LIN.triple)
    assert "THIRD_PARTY_LICENSES.html lacks 'MIT License'" in problems
    assert "THIRD_PARTY_LICENSES.html lacks 'Apache License'" in problems


def test_check_notices_accepts_real_listing_and_ignores_absent():
    assert rv.check_notices(NOTICES_OK.encode()) == []
    assert rv.check_notices(None) == []  # reported by check_layout


def test_pe_exports_parses_names():
    assert rv.pe_exports(fake_pe(["clap_entry", "other"])) == {"clap_entry", "other"}


def test_pe_exports_empty_when_no_table():
    image = bytearray(fake_pe(["x"]))
    struct.pack_into("<I", image, 0x40 + 24 + 112, 0)
    assert rv.pe_exports(bytes(image)) == set()


def test_pe_exports_rejects_non_pe():
    with pytest.raises(ValueError):
        rv.pe_exports(b"ELF....")


def test_has_symbol_elf_style_requires_whole_name():
    assert rv.has_symbol(b"\0clap_entry\0", "clap_entry")
    assert rv.has_symbol(b"\0_GetPluginFactory\0", "GetPluginFactory")
    assert not rv.has_symbol(b"\0my_clap_entry_x\0", "clap_entry")
    assert not rv.has_symbol(b"nothing here", "clap_entry")


def test_windows_archive_ok(tmp_path):
    archive = build(
        tmp_path, WIN, clap=fake_pe(["clap_entry"]), vst3=fake_pe(["GetPluginFactory", "InitDll"])
    )
    assert rv.verify(archive) == []
    assert rv.main([str(archive)]) == 0


def test_linux_and_macos_archives_ok(tmp_path):
    assert rv.verify(build(tmp_path / "l", LIN)) == []
    assert rv.verify(build(tmp_path / "m", MAC)) == []


def test_windows_missing_export_detected(tmp_path):
    archive = build(tmp_path, WIN, clap=fake_pe(["wrong"]), vst3=fake_pe(["GetPluginFactory"]))
    problems = rv.verify(archive)
    assert problems == ["CLAP module does not export clap_entry"]


def test_non_pe_windows_module_reported(tmp_path):
    archive = build(tmp_path, WIN, clap=fake_pe(["clap_entry"]), vst3=b"MZ\0\0")
    assert any("VST3 module unreadable" in p for p in rv.verify(archive))


def test_missing_vst3_binary_detected(tmp_path):
    archive = build(tmp_path, WIN, clap=fake_pe(["clap_entry"]), vst3=fake_pe(["GetPluginFactory"]))
    files = rv.read_archive(archive)
    paths = [p for p in files if not p.endswith("x86_64-win/nd-matrix-1.vst3")]
    problems = rv.check_layout(paths, WIN)
    assert problems == ["missing nd-matrix-1.vst3/Contents/x86_64-win/nd-matrix-1.vst3"]


def test_forbidden_paths_and_types_detected():
    stem = "p"
    base = [f"{stem}/{rel}" for rel in rv.expected_files(LIN)]
    extra = [f"{stem}/.reference/x.txt", f"{stem}/presets/a.ns4p", f"{stem}/fw/os.bin"]
    problems = rv.check_layout(base + extra, LIN)
    assert len(problems) == 3
    assert any(".reference" in p for p in problems)
    assert any("a.ns4p" in p for p in problems)
    assert any("os.bin" in p for p in problems)


def test_multiple_top_folders_detected():
    problems = rv.check_layout(["a/x", "b/y"], LIN)
    assert "one top-level folder" in problems[0]


def test_backslash_paths_detected():
    paths = [f"p/{rel}" for rel in rv.expected_files(LIN)] + ["p/dir\\file"]
    assert any("backslash" in p for p in rv.check_layout(paths, LIN))


def test_manufacturer_name_in_binary_detected(tmp_path):
    archive = build(tmp_path, LIN, clap=b"\0clap_entry\0 Clavia ")
    assert any("manufacturer name" in p for p in rv.verify(archive))


def test_readme_must_disclaim_daw_verification(tmp_path):
    archive = build(tmp_path, LIN, readme="all good")
    assert "README.txt must state the plugin is not DAW-verified" in rv.verify(archive)


def test_sha256_mismatch_and_crlf_detected(tmp_path):
    archive = build(tmp_path, LIN)
    sidecar = archive.with_name(archive.name + ".sha256")
    good = sidecar.read_bytes()
    sidecar.write_bytes(good.replace(b"\n", b"\r\n"))
    assert any("CR line endings" in p for p in rv.check_sha256(archive))
    sidecar.write_bytes(b"0" * 64 + b"  " + archive.name.encode() + b"\n")
    assert any("does not match" in p for p in rv.check_sha256(archive))
    sidecar.unlink()
    assert rv.check_sha256(archive) == [f"missing {sidecar.name}"]


def test_detect_platform_from_name_and_unknown(tmp_path):
    assert rv.detect_platform(tmp_path / "x-v1-macos-arm64.tar.gz") is MAC
    assert rv.detect_platform(tmp_path / "x", "x86_64-pc-windows-msvc") is WIN
    with pytest.raises(ValueError):
        rv.detect_platform(tmp_path / "unknown.zip")


def test_main_reports_problems(tmp_path, capsys):
    archive = build(tmp_path, LIN, clap=b"nothing")
    assert rv.main([str(archive)]) == 1
    assert "does not export clap_entry" in capsys.readouterr().err


def test_main_unreadable_archive(tmp_path, capsys):
    bad = tmp_path / "nd-matrix-1-v1-linux-x86_64.tar.gz"
    bad.write_bytes(b"not gzip")
    assert rv.main([str(bad)]) == 1


def test_linux_archive_without_runtime_notices_fails_release(tmp_path):
    archive = build(tmp_path, LIN)
    assert rv.verify(archive, LIN.triple) == []
    _drop(archive, "/licenses/appimage-runtime/libfuse-LGPL-2.1.txt")
    problems = rv.verify(archive, LIN.triple)
    assert "missing licenses/appimage-runtime/libfuse-LGPL-2.1.txt" in problems


def test_runtime_notices_empty_or_wrong_libfuse_text_fail_release():
    base = rv.APPIMAGE_RUNTIME_DIR
    rel = {f"{base}/{n}": b"text" for n in rv.APPIMAGE_RUNTIME_FILES}
    assert any("not the GNU LGPL" in p for p in rv.check_runtime_notices(rel))
    rel[f"{base}/libfuse-LGPL-2.1.txt"] = b"GNU LESSER GENERAL PUBLIC LICENSE"
    assert rv.check_runtime_notices(rel) == []
    rel[f"{base}/musl-LICENSE.txt"] = b"  \n"
    assert rv.check_runtime_notices(rel) == [f"{base}/musl-LICENSE.txt is empty"]


@pytest.mark.parametrize("platform", [WIN, MAC])
def test_runtime_notices_not_required_outside_linux(tmp_path, platform):
    assert not any(f.startswith("licenses/") for f in rv.expected_files(platform))


def test_linux_release_without_sources_asset_fails(tmp_path):
    archive = build(tmp_path, LIN)
    asset = archive.with_name(rrs.sources_name_for_archive(archive.name))
    asset.unlink()
    problems = rv.verify(archive, LIN.triple)
    assert any("appimage-runtime-sources.tar.gz (LGPL-2.1" in p for p in problems)


def test_sources_asset_must_be_complete_and_checksummed(tmp_path):
    archive = build(tmp_path, LIN)
    write_sources_asset(archive, skip=("fuse-3.15.0.tar.xz",))
    problems = rv.verify(archive, LIN.triple)
    assert any("lacks fuse-3.15.0.tar.xz" in p for p in problems)
    asset = archive.with_name(rrs.sources_name_for_archive(archive.name))
    asset.with_name(asset.name + ".sha256").unlink()
    assert any(
        p.startswith("missing") and p.endswith(".sha256") for p in rv.verify(archive, LIN.triple)
    )


def test_sources_asset_not_required_for_windows_and_macos(tmp_path):
    for platform in (WIN, MAC):
        assert rv.verify(build(tmp_path / platform.name, platform), platform.triple) == []


def test_sources_name_for_archive():
    assert (
        rrs.sources_name_for_archive("nd-matrix-1-v1.2.3-linux-x86_64.tar.gz")
        == "nd-matrix-1-v1.2.3-appimage-runtime-sources.tar.gz"
    )
    assert rrs.sources_name_for_archive("nd-matrix-1-v1.2.3-windows-x86_64.zip") is None


@pytest.mark.parametrize("platform", [WIN, LIN, MAC])
def test_shipped_binaries_and_bundles_have_no_vendor_prefix(tmp_path, platform):
    """ns4-b9rn: a clean archive passes, and none of the shipped names starts with `nord-`."""
    archive = build(tmp_path, platform)
    names = [p for p in rv.read_archive(archive)]
    assert rv.check_vendor_names(archive.name, names) == []
    assert not any(part.lower().startswith("nord-") for n in names for part in n.split("/"))
    assert not any(b.lower().startswith("nord-") for b in rp.BINS)


def test_vendor_prefixed_binary_fails_release(tmp_path, monkeypatch):
    """The pre-rename `nord-plugin-standalone` is rejected even when it is the expected binary."""
    old = ("ns4", "nd-matrix-1", "nord-plugin-standalone")
    monkeypatch.setattr(rp, "BINS", old)
    monkeypatch.setattr(rv, "BINS", old)  # layout check now expects it, so only the new check fires
    archive = build(tmp_path, WIN)
    problems = rv.verify(archive, WIN.triple)
    assert problems and all("nord-plugin-standalone.exe" in p and "'nord-'" in p for p in problems)


def test_vendor_prefixed_bundle_fails_release(tmp_path):
    """A `nord-plugin.vst3` bundle folder is rejected, whatever is inside."""
    import io
    import tarfile

    archive = build(tmp_path, LIN)
    files = rv.read_archive(archive)
    top = next(iter(files)).split("/", 1)[0]
    files[f"{top}/nord-plugin.vst3/Contents/x86_64-linux/nord-plugin.so"] = b"x"
    with tarfile.open(archive, "w:gz") as tf:
        for name, data in files.items():
            info = tarfile.TarInfo(name)
            info.size = len(data)
            tf.addfile(info, io.BytesIO(data))
    rp.write_sha256(archive)
    problems = rv.verify(archive, LIN.triple)
    assert any("'nord-plugin.vst3'" in p for p in problems)
    assert any("'nord-plugin.so'" in p for p in problems)


def test_vendor_prefixed_archive_name_fails_release():
    problems = rv.check_vendor_names("Nord-Stage-4-v1.zip", ["Nord-Stage-4-v1/ns4.exe"])
    assert any("archive name" in p for p in problems)
    assert not any("'ns4.exe'" in p for p in problems)
    assert any("'Nord-Stage-4-v1'" in p for p in problems)


def test_check_vendor_names_clean_and_deduplicates():
    assert (
        rv.check_vendor_names("nd-matrix-1-v1-linux.tar.gz", ["top/ns4", "top/nd-matrix-1"]) == []
    )
    paths = ["top/nord-x/a", "top/nord-x/b"]
    assert len(rv.check_vendor_names("ok.zip", paths)) == 1
