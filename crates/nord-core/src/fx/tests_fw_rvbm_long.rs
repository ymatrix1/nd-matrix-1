//! Long-window parity of the multi-tap reverb (types 2..11) with the oracle: 30,000 calls of an
//! impulse (amount 127) and of noise (amount 63) per type, longer than every tail (the first
//! golden window of `tests_fw_rvbm_golden.rs` is 3,600 calls and ends before the Hall and
//! Cathedral tails start). A wrap error of the 3,140 word ring or the 20,480 word SDRAM line, as
//! the CHOR port had (D133), would show here and not in the short window.

use super::fw_ext40::E40;
use super::tests_fw_rvbm_golden::{fnv, run};
use super::tests_fw_rvbm_long_data::GOLDEN_LONG;

#[test]
fn e40_port_equals_the_oracle_over_the_whole_tail() {
    for g in &GOLDEN_LONG {
        let out = run::<E40>(g, g.calls);
        let flat = || out.iter().flatten().copied();
        let tag = format!("type {} noise {}", g.rvb_type, g.noise);
        assert_eq!(
            flat().filter(|w| w & 0x7FFF_FFFF != 0).count(),
            g.nonzero,
            "{tag}"
        );
        assert_eq!(fnv(flat()), g.hash, "{tag}: hash of every output word");
    }
}
