//! Per-engine golden renders (the former `render_hash` example, now only this test).
//!
//! Five segments (`tests/support/render_segments.rs`): the full engine, the synth, the organ, the
//! piano and the FX sweep. Their concatenation is the `render_hash` stream. Two checks per
//! segment:
//!
//! * fingerprint (always): byte length, all samples finite, peak and RMS within 0.1 % of the
//!   recorded values. A real behaviour change moves them; platform `libm` last-bit differences
//!   (the CI runs Linux, Windows and macOS) do not.
//! * SHA-256 (exact; set `NS4_GOLDEN_EXACT=1` and run with `--release`): the byte-for-byte
//!   golden. Recorded on Windows x86_64 in a release build; a debug build gives other FX bits
//!   (segment `fx`), so the exact check belongs to release only. Other operating systems are not
//!   verified (platform `libm`), hence opt-in.
//!
//! Re-baselining (only with a PR that says which segment changed and why, `docs/DECISIONS.md` D22):
//!
//! 1. `NS4_GOLDEN_PRINT=1 cargo test --release -p nord-core --test golden_render -- --nocapture`
//!    prints one line per segment (`GOLDEN <name> <bytes> <peak> <rms> <sha256>`) and
//!    `GOLDEN whole <sha256>` (the `render_hash` value `docs/TECH_DEBT.md` and `docs/DECISIONS.md`
//!    quote).
//! 2. Copy the lines into `GOLDEN` and `WHOLE` below, run
//!    `NS4_GOLDEN_EXACT=1 cargo test --release -p nord-core --test golden_render`, and update
//!    the whole-file hash in the docs.

#[path = "support/render_segments.rs"]
mod render_segments;

use render_segments::{segments, sha256_hex, SEGMENT_NAMES};

/// `(name, bytes, peak, rms, sha256)` per segment.
const GOLDEN: [(&str, usize, f64, f64, &str); 5] = [
    (
        "engine",
        480000,
        1.000000,
        0.214985,
        "631feec6d2a405a61f5703f6fd3a54e0720092688d7bf145849d6251c6a7fdb0",
    ),
    (
        "synth",
        768000,
        0.424988,
        0.140422,
        "808bbc1bd59145ab5a1748e1949c0c8ee7a318128707d764bd967e52de8b1bb6",
    ),
    (
        "organ",
        96000,
        0.773433,
        0.196090,
        "e9f01099f60c4b9c5837b40f1fafa0f627d066a1e32f32f57a103cf38b3d3485",
    ),
    (
        "piano",
        96000,
        0.390195,
        0.140492,
        "def4da300a454632f1ff69d338af0ddaec11adcc4f0d7b667d2a327374bf164e",
    ),
    (
        "fx",
        800000,
        0.996211,
        0.226648,
        "574417ff36d30c50b44276e0497d57b8740b3cbc25dd174953dcdf2a7f1cc213",
    ),
];

/// Whole-file SHA-256 of `render_hash` (the five segments joined).
const WHOLE: &str = "eb5f8f499d8987bbced2d6b1ee3ad30200202792821421e0b28e2feaea29bbcc";

/// Peak and RMS of little-endian f32 samples; `None` when one is not finite.
fn stats(bytes: &[u8]) -> Option<(f64, f64)> {
    let (mut peak, mut sum, mut n) = (0f64, 0f64, 0usize);
    for c in bytes.as_chunks::<4>().0 {
        let x = f64::from(f32::from_le_bytes(*c));
        if !x.is_finite() {
            return None;
        }
        peak = peak.max(x.abs());
        sum += x * x;
        n += 1;
    }
    Some((peak, (sum / n.max(1) as f64).sqrt()))
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-3 * b.abs().max(1e-9)
}

/// Renders on a thread with a large stack: the FX chains are big values in a debug build.
fn rendered() -> Vec<(&'static str, Vec<u8>)> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(segments)
        .unwrap()
        .join()
        .unwrap()
}

#[test]
fn every_engine_segment_matches_its_golden_fingerprint() {
    let segs = rendered();
    assert_eq!(
        segs.iter().map(|s| s.0).collect::<Vec<_>>(),
        SEGMENT_NAMES.to_vec()
    );
    let print = std::env::var_os("NS4_GOLDEN_PRINT").is_some();
    let exact = std::env::var_os("NS4_GOLDEN_EXACT").is_some();
    let mut whole = Vec::new();
    for ((name, bytes), (gname, glen, gpeak, grms, ghash)) in segs.iter().zip(GOLDEN) {
        let (peak, rms) = stats(bytes).unwrap_or_else(|| panic!("{name}: non-finite sample"));
        let hash = sha256_hex(bytes);
        if print {
            println!("GOLDEN {name} {} {peak:.6} {rms:.6} {hash}", bytes.len());
            whole.extend_from_slice(bytes);
            continue; // re-baseline mode: print only
        }
        whole.extend_from_slice(bytes);
        assert_eq!(*name, gname);
        assert_eq!(bytes.len(), glen, "{name}: length");
        assert!(near(peak, gpeak), "{name}: peak {peak} vs {gpeak}");
        assert!(near(rms, grms), "{name}: rms {rms} vs {grms}");
        if exact {
            assert_eq!(
                hash, ghash,
                "{name}: SHA-256 (re-baseline, see the file docs)"
            );
        }
    }
    if print {
        println!("GOLDEN whole {}", sha256_hex(&whole));
        return;
    }
    if exact {
        assert_eq!(sha256_hex(&whole), WHOLE, "whole render_hash");
    }
}

#[test]
fn sha256_matches_the_standard_vectors() {
    assert_eq!(
        sha256_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(sha256_hex(&[b'a'; 1000]).len(), 64);
}
