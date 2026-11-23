"""Check that the project license metadata is consistent.

The project is source-available under PolyForm Noncommercial 1.0.0 (D231).
This module reads the root ``LICENSE``, the workspace ``Cargo.toml`` and every
crate manifest, and reports the effective license and publish flag per crate.
"""

from __future__ import annotations

import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SPDX_ID = "PolyForm-Noncommercial-1.0.0"
LICENSE_HEADER = "# PolyForm Noncommercial License 1.0.0"


def read_toml(path: Path) -> dict:
    """Parse a TOML file."""
    with path.open("rb") as handle:
        return tomllib.load(handle)


def license_file_ok(root: Path = ROOT) -> bool:
    """True when ``LICENSE`` exists and starts with the PolyForm Noncommercial header."""
    path = root / "LICENSE"
    if not path.is_file():
        return False
    return path.read_text(encoding="utf-8").startswith(LICENSE_HEADER)


def _resolve(value: object, workspace_value: object) -> object:
    """Resolve a manifest field that may be ``{workspace = true}``."""
    if isinstance(value, dict) and value.get("workspace") is True:
        return workspace_value
    return value


def crate_settings(root: Path = ROOT) -> dict[str, tuple[object, object]]:
    """Map each workspace crate name to its effective ``(license, publish)``."""
    workspace = read_toml(root / "Cargo.toml")
    ws_pkg = workspace.get("workspace", {}).get("package", {})
    result: dict[str, tuple[object, object]] = {}
    for member in workspace["workspace"]["members"]:
        package = read_toml(root / member / "Cargo.toml")["package"]
        result[package["name"]] = (
            _resolve(package.get("license"), ws_pkg.get("license")),
            _resolve(package.get("publish"), ws_pkg.get("publish")),
        )
    return result


def problems(root: Path = ROOT) -> list[str]:
    """List every license inconsistency found under ``root``."""
    found: list[str] = []
    if not license_file_ok(root):
        found.append("LICENSE missing or not the PolyForm Noncommercial 1.0.0 text")
    for name, (license_id, publish) in sorted(crate_settings(root).items()):
        if license_id != SPDX_ID:
            found.append(f"{name}: license is {license_id!r}, expected {SPDX_ID!r}")
        if publish is not False:
            found.append(f"{name}: publish is {publish!r}, expected false")
    py_license = read_toml(root / "pyproject.toml")["project"].get("license")
    if py_license != SPDX_ID:
        found.append(f"pyproject.toml: license is {py_license!r}, expected {SPDX_ID!r}")
    return found
