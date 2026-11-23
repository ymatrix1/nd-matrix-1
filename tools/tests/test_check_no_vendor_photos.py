import check_no_vendor_photos as c


def test_forbidden_names_flagged():
    bad = c.find_violations(["crates/nord-gui/assets/panel_texture.jpg", "x/keyboard_full.jpg"])
    assert len(bad) == 2


def test_jpeg_under_crates_or_assets_flagged_unless_allowlisted(monkeypatch):
    assert c.find_violations(["assets/a.JPG"])
    assert c.find_violations(["docs/img/a.jpg"]) == []
    monkeypatch.setattr(c, "JPEG_ALLOWLIST", {"assets/a.jpg"})
    assert c.find_violations(["assets/a.jpg"]) == []


def test_png_is_fine():
    assert c.find_violations(["crates/nord-gui/assets/baked/panel_plate_2x.png"]) == []


def test_include_bytes_jpeg_flagged(tmp_path):
    d = tmp_path / "crates" / "x" / "src"
    d.mkdir(parents=True)
    (d / "a.rs").write_text('const B: &[u8] = include_bytes!("../assets/p.jpg");\n')
    (d / "b.rs").write_text('const B: &[u8] = include_bytes!("../assets/p.png");\n')
    bad = c.find_violations(["crates/x/src/a.rs", "crates/x/src/b.rs"], root=tmp_path)
    assert bad == ["crates/x/src/a.rs: include_bytes! of a JPEG"]


def test_current_tree_is_clean():
    assert c.find_violations(c.tracked_files()) == []
