import json

import sweep_flip_compare as sf


def line(entry, default, firmware, arp=None, wheel=0.0):
    layer = {"arp_enabled": bool(arp), "arp": {"mode": 0, "range": arp or 0}}
    return json.dumps(
        {
            "entry": entry,
            "renders": {"default": default, "firmware": firmware},
            "program": {
                "synth": {"layer_a": layer},
                "organ": {"m": {"aftertouch": 0.0, "ctrl_pedal": 0.0, "wheel": wheel}},
            },
        }
    )


def write(tmp_path, name, lines):
    p = tmp_path / name
    p.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return p


def test_arp_layers_and_morph_slot_detection():
    prog = json.loads(line("Program/Bank A/X.ns4p", 1, 1, arp=47, wheel=0.5))["program"]
    assert sf.arp_layers(prog) == ["A:0:47"]
    assert sf.has_morph_slot(prog)
    assert not sf.has_morph_slot({"organ": {"m": {"aftertouch": 0, "ctrl_pedal": 0, "wheel": 0}}})
    assert sf.arp_layers({"synth": {}}) == []


def test_compare_flags_changed_programs_and_summarizes(tmp_path):
    old = write(
        tmp_path,
        "old.jsonl",
        [line("Program/Bank A/Arp.ns4p", 1, 1, arp=47), line("Program/Bank A/Plain.ns4p", 2, 2)],
    )
    new = write(
        tmp_path,
        "new.jsonl",
        [line("Program/Bank A/Arp.ns4p", 9, 1, arp=47), line("Program/Bank A/Plain.ns4p", 2, 2)],
    )
    rows = sf.compare(sf.load(old), sf.load(new))
    assert [r["program"] for r in rows] == ["Bank A/Arp", "Bank A/Plain"]
    assert rows[0]["default_changed"] and not rows[0]["firmware_changed"]
    assert not rows[1]["default_changed"]
    s = sf.summarize(rows)
    assert s["programs"] == 2 and s["changed"] == 1 and s["changed_with_arp"] == 1
    assert s["changed_without_arp_or_morph"] == 0 and s["arp_programs"] == 1


def test_main_writes_the_csv(tmp_path, capsys):
    old = write(tmp_path, "o.jsonl", [line("Program/Bank A/A.ns4p", 1, 1)])
    new = write(tmp_path, "n.jsonl", [line("Program/Bank A/A.ns4p", 1, 3)])
    out = tmp_path / "o.csv"
    assert sf.main([str(old), str(new), "--csv", str(out)]) == 0
    assert out.read_text(encoding="utf-8").splitlines()[0].startswith("program,arp_layers")
    assert json.loads(capsys.readouterr().out)["changed"] == 1
