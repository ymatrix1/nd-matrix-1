//! The two views of the peripheral models that `exact_diff` can read. OWNER: W1-VERIFY.
//!
//! A model keeps its registers behind its own API and most models derive neither `PartialEq` nor
//! `Debug` (`Sc57x`, `SpiBoot`, `Sec`, `Spi2`, `L2`), so the comparator cannot open them. It
//! reads two things instead:
//!
//! - which models are attached (`periph`), so a handler that takes the peripheral box and does
//!   not put it back, or a core configured differently, shows up;
//! - the access census (`periph_log`): every address outside SRAM with its read and write counts,
//!   the last value written, the last value read and the pc of the last access. A store of the
//!   right address with the wrong value to a model-owned register ends in `last_write`, so the
//!   census is the comparator of the peripheral traffic. It exists only when the host called
//!   `Core::enable_access_log`.
//!
//! The limit: the internal state of a model is not compared, and a core without the census has no
//! peripheral comparator beyond the presence flags and the read and write counters (`device_io`).
//! Lockstep on a core with peripherals must call `enable_access_log` on both cores, and shadow
//! mode and `UopChecked` skip such cores (design risk 6; `CoreSnap` holds no peripheral state).
use crate::core::periph::access_log::Access;
use crate::core::Core;

/// Whether each model is attached: `sc57x`, `boot`, `epdma`, `epchain`, `ringdma`.
pub(super) fn models(c: &Core) -> [bool; 5] {
    match c.periph.as_deref() {
        Some(p) => [
            p.sc57x.is_some(),
            p.boot.is_some(),
            p.epdma.is_some(),
            p.epchain.is_some(),
            p.ringdma.is_some(),
        ],
        None => [false; 5],
    }
}

/// The census as `(address, counts and last values)` in ascending address order, or `None` when
/// the census is off (an empty census is `Some(vec![])`, which differs from `None`).
pub(super) fn log(c: &Core) -> Option<Vec<(u32, Access)>> {
    let l = c.periph.as_ref()?.log.as_ref()?;
    Some(l.entries().map(|(addr, a)| (addr, *a)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::periph::epdma::RANGE_START;
    use crate::core::uop::verify::compare::exact_diff;

    #[test]
    fn a_core_without_peripherals_shows_no_model_and_no_census() {
        let c = Core::new(&[]);
        assert_eq!((models(&c), log(&c)), ([false; 5], None));
    }

    #[test]
    fn each_model_and_the_census_show_up_in_their_own_slot() {
        let enable: [fn(&mut Core); 5] = [
            |c| c.enable_sc57x(0),
            |c| c.enable_spi_boot(Vec::new()),
            Core::enable_epdma,
            Core::enable_epchain,
            Core::enable_ringdma,
        ];
        for (slot, f) in enable.iter().enumerate() {
            let mut c = Core::new(&[]);
            f(&mut c);
            let mut want = [false; 5];
            want[slot] = true;
            assert_eq!(models(&c), want, "slot {slot}");
            assert_eq!(log(&c), None, "a model is not the census");
        }
        let mut c = Core::new(&[]);
        c.enable_access_log();
        assert_eq!((models(&c), log(&c)), ([false; 5], Some(Vec::new())));
    }

    #[test]
    fn the_census_lists_addresses_ascending_with_counts_and_last_values() {
        let mut c = Core::new(&[]);
        c.enable_access_log();
        c.pc = 0x12_0040;
        c.dm_write(0x3000_0020, 7).unwrap();
        c.dm_write(0x3000_0020, 8).unwrap();
        c.dm_read(0x3000_0010).unwrap();
        let l = log(&c).unwrap();
        let addrs: Vec<u32> = l.iter().map(|e| e.0).collect();
        assert_eq!(addrs, [0x3000_0010, 0x3000_0020]);
        let w = l[1].1;
        assert_eq!((w.writes, w.last_write, w.last_pc), (2, Some(8), 0x12_0040));
        assert_eq!((l[0].1.reads, l[0].1.last_read), (1, Some(0)));
    }

    /// Two cores with the range-transfer model and the census on, the first of which stored
    /// `first` and the second `second` to the model's range-start register.
    fn stored(first: u32, second: u32) -> (Core, Core) {
        let make = |v: u32| {
            let mut c = Core::new(&[]);
            c.enable_epdma();
            c.enable_access_log();
            c.dm_write(RANGE_START, v).unwrap();
            c
        };
        (make(first), make(second))
    }

    #[test]
    fn a_wrong_value_stored_to_a_model_register_is_seen_through_the_census() {
        let (a, b) = stored(5, 5);
        assert_eq!(exact_diff(&a, &b), None);
        // the model took the store, so the device store and every counter agree
        let (a, b) = stored(5, 6);
        assert_eq!((a.device_writes, b.device_writes), (1, 1));
        assert!(a.device.is_empty() && b.device.is_empty());
        let d = exact_diff(&a, &b).expect("the census holds the last value written");
        assert_eq!(d.field, "periph_log", "{d}");
    }

    #[test]
    fn a_model_attached_to_one_core_only_is_a_difference() {
        let (mut a, b) = (Core::new(&[]), Core::new(&[]));
        a.enable_ringdma();
        assert_eq!(exact_diff(&a, &b).map(|d| d.field), Some("periph"));
    }
}
