//! Long-window golden fingerprints of the multi-tap reverb: oracle output of `0x120FDB` over
//! 30,000 calls (2.76 s at the DSP frame rate), longer than every tail of the nine types.
//!
//! Derived numbers of `python tools/fx_rvbmulti.py golden <core2_sram.bin> --calls 30000`, committed
//! as `docs/research/data/fx-rvbmulti-golden-long.csv`; same columns as `tests_fw_rvbm_golden_data.rs`.

use super::tests_fw_rvbm_golden_data::Golden;

pub(super) const GOLDEN_LONG: [Golden; 18] = [
    Golden {
        rvb_type: 2,
        noise: false,
        amount: 127,
        calls: 30000,
        hash: 0x0fc218b0b630f84f,
        nonzero: 61711,
        first: 848,
    },
    Golden {
        rvb_type: 2,
        noise: true,
        amount: 63,
        calls: 30000,
        hash: 0xe97621c3b7431027,
        nonzero: 240000,
        first: 0,
    },
    Golden {
        rvb_type: 3,
        noise: false,
        amount: 127,
        calls: 30000,
        hash: 0x30812dc64227cecd,
        nonzero: 61606,
        first: 1912,
    },
    Golden {
        rvb_type: 3,
        noise: true,
        amount: 63,
        calls: 30000,
        hash: 0xa97db40d20121aff,
        nonzero: 240000,
        first: 0,
    },
    Golden {
        rvb_type: 4,
        noise: false,
        amount: 127,
        calls: 30000,
        hash: 0x04d8544957d33bf8,
        nonzero: 80480,
        first: 1061,
    },
    Golden {
        rvb_type: 4,
        noise: true,
        amount: 63,
        calls: 30000,
        hash: 0xfc237dbb6c1e8d3e,
        nonzero: 240000,
        first: 0,
    },
    Golden {
        rvb_type: 5,
        noise: false,
        amount: 127,
        calls: 30000,
        hash: 0x6fba994995676b61,
        nonzero: 78982,
        first: 2557,
    },
    Golden {
        rvb_type: 5,
        noise: true,
        amount: 63,
        calls: 30000,
        hash: 0x013aa5b4f0d8740b,
        nonzero: 240000,
        first: 0,
    },
    Golden {
        rvb_type: 7,
        noise: false,
        amount: 127,
        calls: 30000,
        hash: 0x737b5898f35635a5,
        nonzero: 103208,
        first: 1219,
    },
    Golden {
        rvb_type: 7,
        noise: true,
        amount: 63,
        calls: 30000,
        hash: 0xa79bd3d0cbeba4c3,
        nonzero: 240000,
        first: 0,
    },
    Golden {
        rvb_type: 8,
        noise: false,
        amount: 127,
        calls: 30000,
        hash: 0x88a5a3a40d16783e,
        nonzero: 137562,
        first: 2157,
    },
    Golden {
        rvb_type: 8,
        noise: true,
        amount: 63,
        calls: 30000,
        hash: 0xad7ef302102f94a9,
        nonzero: 240000,
        first: 0,
    },
    Golden {
        rvb_type: 9,
        noise: false,
        amount: 127,
        calls: 30000,
        hash: 0xe354f3493c6d6e2d,
        nonzero: 137921,
        first: 2157,
    },
    Golden {
        rvb_type: 9,
        noise: true,
        amount: 63,
        calls: 30000,
        hash: 0xf38482a4a6bdcd00,
        nonzero: 240000,
        first: 0,
    },
    Golden {
        rvb_type: 10,
        noise: false,
        amount: 127,
        calls: 30000,
        hash: 0xad50276f7158147a,
        nonzero: 139599,
        first: 3025,
    },
    Golden {
        rvb_type: 10,
        noise: true,
        amount: 63,
        calls: 30000,
        hash: 0xf8f971f6e3c4d26b,
        nonzero: 240000,
        first: 0,
    },
    Golden {
        rvb_type: 11,
        noise: false,
        amount: 127,
        calls: 30000,
        hash: 0x3dd438e0b94ca887,
        nonzero: 138668,
        first: 3025,
    },
    Golden {
        rvb_type: 11,
        noise: true,
        amount: 63,
        calls: 30000,
        hash: 0x4208036468018b1a,
        nonzero: 240000,
        first: 0,
    },
];

#[test]
fn the_long_table_covers_the_nine_types_with_both_signals() {
    assert_eq!(GOLDEN_LONG.len(), 18);
    for t in [2, 3, 4, 5, 7, 8, 9, 10, 11] {
        assert_eq!(GOLDEN_LONG.iter().filter(|g| g.rvb_type == t).count(), 2);
    }
    assert!(GOLDEN_LONG.iter().all(|g| g.calls == 30_000));
}

/// The table equals the committed CSV (`docs/research/data/fx-rvbmulti-golden-long.csv`). The CSV
/// itself needs the local FX image and the oracle to regenerate, so CI cannot check it against
/// the oracle.
#[test]
fn the_table_equals_the_committed_csv() {
    let csv = include_str!("../../../../docs/research/data/fx-rvbmulti-golden-long.csv");
    let rows: Vec<Vec<&str>> = csv
        .lines()
        .skip(1)
        .map(|l| l.trim().split(',').collect())
        .collect();
    assert_eq!(rows.len(), GOLDEN_LONG.len());
    for (r, g) in rows.iter().zip(&GOLDEN_LONG) {
        assert_eq!(r[0].parse::<usize>().unwrap(), g.rvb_type);
        assert_eq!(r[1] == "noise", g.noise);
        assert_eq!(r[2].parse::<usize>().unwrap(), g.amount);
        assert_eq!(r[3].parse::<usize>().unwrap(), g.calls);
        assert_eq!(u64::from_str_radix(&r[4][2..], 16).unwrap(), g.hash);
        assert_eq!(r[5].parse::<usize>().unwrap(), g.nonzero);
        assert_eq!(r[6].parse::<i64>().unwrap(), g.first);
    }
}
