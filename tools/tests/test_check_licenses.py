import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import check_licenses as c  # noqa: E402
import pytest  # noqa: E402


@pytest.mark.parametrize(
    "expr",
    [
        "MIT",
        "MIT OR Apache-2.0",
        "ISC",
        "Apache-2.0 WITH LLVM-exception",
        "(MIT OR Apache-2.0) AND Unicode-3.0",
        "MIT/Apache-2.0",
        "MIT OR LGPL-2.1-or-later OR BSD-3-Clause",
        "MIT OR GPL-3.0",
        "Zlib",
        "Unlicense OR MIT",
    ],
)
def test_permissive_expressions_pass(expr):
    assert c.expression_is_permissive(expr)


@pytest.mark.parametrize(
    "expr",
    [
        "GPL-3.0",
        "GPL-3.0-only",
        "GPL-2.0-or-later",
        "AGPL-3.0",
        "LGPL-2.1-only",
        "LGPL-3.0+",
        "MPL-2.0",
        "SSPL-1.0",
        "EUPL-1.2",
        "MIT AND GPL-3.0",
        "GPL-3.0 OR MPL-2.0",
        "(MIT OR Apache-2.0) AND MPL-2.0",
    ],
)
def test_copyleft_expressions_fail(expr):
    assert not c.expression_is_permissive(expr)


def test_malformed_expression_raises():
    with pytest.raises(ValueError):
        c.expression_is_permissive("MIT OR")
    with pytest.raises(ValueError):
        c.expression_is_permissive("(MIT OR Apache-2.0")


def pkg(name, license_, source="registry+https://github.com/rust-lang/crates.io-index"):
    return {
        "id": f"{name} 1.0.0",
        "name": name,
        "version": "1.0.0",
        "license": license_,
        "source": source,
    }


def meta():
    packages = [
        pkg("nord-plugin", "MIT", source=None),
        pkg("nih_plug", "ISC"),
        pkg("vst3-sys", "GPL-3.0"),
        pkg("vst3", "MIT OR Apache-2.0"),
        pkg("testonly", "GPL-3.0"),
        pkg("nolicense", None),
    ]
    ids = {p["name"]: p["id"] for p in packages}

    def node(name, deps):
        return {
            "id": ids[name],
            "deps": [{"pkg": ids[d], "dep_kinds": [{"kind": kind}]} for d, kind in deps],
        }

    nodes = [
        node("nord-plugin", [("nih_plug", None), ("testonly", "dev")]),
        node("nih_plug", [("vst3", None), ("nolicense", "build")]),
        node("vst3", []),
        node("vst3-sys", []),
        node("testonly", []),
        node("nolicense", []),
    ]
    return {
        "packages": packages,
        "workspace_members": [ids["nord-plugin"]],
        "resolve": {"nodes": nodes},
    }


def test_shipped_packages_skip_dev_workspace_and_unreachable():
    names = [p["name"] for p in c.shipped_packages(meta(), "nord-plugin")]
    assert names == ["nih_plug", "nolicense", "vst3"]


def test_violations_flag_missing_license_only_for_shipped_crates():
    bad = c.violations(c.shipped_packages(meta(), "nord-plugin"))
    assert bad == ["nolicense 1.0.0: no SPDX license expression"]


def test_violations_catch_gpl_when_reachable():
    m = meta()
    m["resolve"]["nodes"][2]["deps"].append(
        {"pkg": "vst3-sys 1.0.0", "dep_kinds": [{"kind": None}]}
    )
    bad = c.violations(c.shipped_packages(m, "nord-plugin"))
    assert "vst3-sys 1.0.0: copyleft-only license 'GPL-3.0'" in bad


def test_unknown_package_raises():
    with pytest.raises(ValueError):
        c.shipped_packages(meta(), "nope")
