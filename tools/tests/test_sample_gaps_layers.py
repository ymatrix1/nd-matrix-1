import sample_gaps_layers as g


def test_prepared_records_keeps_only_sy_prepare_rows():
    rows = [
        "key 1 SY 1 0 0x92414 0x001c07cd 0xc01e11ac",
        "key 2 SY 1 0 0x9244c 0x001c07cd 0xc01e11ac",
        "key 2 SY 1 0 0x9244c 0x001c07cd 0xc01e11ac",
        "key 3 SY 1 0 0x92484 0x001c0794 0xc01e11ac",
        "key 3 ORG 1 0 0x92484 0x001c07cd 0xc01e11ac",
    ]
    assert g.prepared_records(rows) == [0x92414, 0x9244C]


def test_parse_bind():
    assert g.parse_bind("0,2,4") == [0, 2, 4]
    assert g.parse_bind("-") == []
    assert g.parse_bind("") == []
