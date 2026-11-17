"""Tests for `tools/fx_pump_clock_probe.py` (word classification only; the run needs firmware)."""

import fx_pump_clock_probe as probe

# Values of the A:25 run (docs/research/fx-mod-types.md s7.2): pointer at 0xC165F.
FREE = {0xC165C: 0x127200, 0xC165F: 0xC165D, 0xC1660: 0x1, 0xC1666: 0x7FFFFFFF}
SYNC = {0xC165C: 0x127200, 0xC165F: 0xC0C70, 0xC1660: 0x20, 0xC1666: 0x7FFFFFFF}


def test_pointer_and_multiplier_are_found():
    found = probe.pointer_and_n(FREE, SYNC)
    assert found == {
        "pointer_dm": 0xC165F,
        "free_target": 0xC165D,
        "sync_target": 0xC0C70,
        "n_dm": 0xC1660,
        "n": 0x20,
        "free_n": 1,
    }


def test_no_pointer_is_reported_as_none():
    assert probe.pointer_and_n(FREE, FREE) is None
    assert probe.pointer_and_n({}, SYNC) is None
    # Pointer changes but the multiplier word is missing from the MST CLK run.
    assert probe.pointer_and_n(FREE, {0xC165F: 0xC0C70}) is None
