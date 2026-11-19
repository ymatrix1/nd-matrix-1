import sample_gaps_cov as c


def row(phase, dm, w2=c.STATE_PREPARE, chip="SY", path=1):
    return {"phase": phase, "chip": chip, "path": path, "dm": dm, "w2": w2}


def test_voice_of_maps_state_words_only():
    assert c.voice_of(0x92414) == 0
    assert c.voice_of(0x92414 + 0x38 * 2) == 2
    assert c.voice_of(0x92415) is None
    assert c.voice_of(0x92000) is None


def test_prepared_voices_per_phase_ignores_other_rows():
    rows = [
        row("k60", 0x92414),
        row("k60", 0x92414 + 0x38),
        row("k84", 0x92414),
        row("k60", 0x92414, w2=0),
        row("k60", 0x92414 + 0x70, chip="ORG"),
    ]
    v = c.prepared_voices(rows)
    assert v == {"k36": [], "k60": [0, 1], "k84": [0]}


def test_classify_and_summarize():
    assert c.classify(2, {"k60": [0, 1]}) == "all layers at key 60"
    assert c.classify(3, {"k60": [0]}) == "some layers at key 60"
    assert c.classify(1, {"k84": [0]}) == "voices only at other keys"
    assert c.classify(1, {}) == "no voice queued"
    assert c.classify(0, {"k60": [0]}) == "no scene I sample layer"
    s = c.summarize([{"class": "a", "layers": 1}, {"class": "a", "layers": 2}])
    assert s == {"programs": 2, "classes": {"a": 2}, "layers": {1: 1, 2: 1}}
