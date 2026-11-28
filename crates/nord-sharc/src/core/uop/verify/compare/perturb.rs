//! One perturbation per name of [`FIELDS`](super::FIELDS), shared by the comparator and snapshot
//! tests: applying entry `k` to a fresh core must make `exact_diff` report exactly the named
//! field. OWNER: W1-VERIFY. Test-only.
use super::{exact_diff, FIELDS};
use crate::core::Core;

/// A named mutation of one compared field.
pub(in crate::core) type Perturbation = (&'static str, fn(&mut Core));

/// The table, in [`FIELDS`] order. The `sram` entry flips a byte far from the code window, the
/// `sdram` and `device` entries store through the window's own accessors, the `periph` entries
/// attach a model and the census (the snapshot holds no peripheral state, see `snap.rs`).
pub(in crate::core) fn perturbations() -> Vec<Perturbation> {
    use crate::core::interrupts::Serviced;
    use crate::core::types::{LoopFrame, Pending};
    let table: &[Perturbation] = &[
        ("pc", |c| c.pc ^= 1),
        ("steps", |c| c.steps += 1),
        ("r", |c| c.r[3] ^= 1),
        ("s", |c| c.s[3] ^= 1),
        ("i", |c| c.i[3] ^= 1),
        ("m", |c| c.m[3] ^= 1),
        ("l", |c| c.l[3] ^= 1),
        ("b", |c| c.b[3] ^= 1),
        ("f40_x", |c| {
            c.fx.tag[2] = c.r[2];
            c.fx.lo[2] = 1;
        }),
        ("f40_y", |c| {
            c.fx_y.tag[2] = c.s[2];
            c.fx_y.lo[2] = 1;
        }),
        ("fx_x", |c| {
            // stale (tag unequal to R2) with a nonzero byte: the effective value does not move
            c.fx.tag[2] = c.r[2] ^ 1;
            c.fx.lo[2] = 1;
        }),
        ("fx_y", |c| {
            c.fx_y.tag[2] = c.s[2] ^ 1;
            c.fx_y.lo[2] = 1;
        }),
        ("mr", |c| c.mr[1] += 1),
        ("mr_y", |c| c.mr_y[0] -= 1),
        ("fifo", |c| c.flags.fifo.bff ^= 1),
        ("fifo_y", |c| c.flags_y.fifo.wrp = 3),
        ("flags", |c| c.flags.az = true),
        ("flags_y", |c| c.flags_y.tf = true),
        ("mode1", |c| c.mode1 ^= 1 << 3),
        ("mode2", |c| c.mode2 ^= 1),
        ("ustat", |c| c.ustat[1] ^= 1),
        ("imask", |c| c.imask ^= 1),
        ("mmask", |c| c.mmask ^= 1),
        ("astat_extra", |c| c.sys.astat_extra[1] ^= 1),
        ("stky", |c| c.sys.stky[0] ^= 1),
        ("sys_status", |c| c.sys.status.push([1, 2, 3])),
        ("pc_overflow", |c| c.sys.pc_overflow = true),
        ("flags_reg", |c| c.sys.flags_reg ^= 1),
        ("flag_pins", |c| c.sys.flag_pins ^= 1),
        ("px", |c| c.sys.px ^= 1),
        ("cb_wrap", |c| c.cb_wrap.set(c.cb_wrap.get() ^ 1)),
        ("call_stack", |c| c.call_stack.push(7)),
        ("loops", |c| c.loops.push(LoopFrame::default())),
        ("pending", |c| {
            c.pending = Some(Pending {
                slots: 3,
                target: 5,
                call: false,
            });
        }),
        ("irptl", |c| c.irq.irptl ^= 1),
        ("lirptl", |c| c.irq.lirptl ^= 1),
        ("irq_idle", |c| c.irq.idle = true),
        ("irq_ivt_base", |c| c.irq.ivt_base ^= 0x100),
        ("irq_window_lo", |c| c.irq.window_lo ^= 1),
        ("irq_window_len", |c| c.irq.window_len ^= 1),
        ("irq_stall", |c| c.irq.stall += 1),
        ("irq_due", |c| c.irq.due += 1),
        ("irq_taken", |c| c.irq.taken += 1),
        ("irq_serviced", |c| {
            c.irq.serviced.push(Serviced {
                vector: 3,
                status_pushed: false,
            });
        }),
        ("irq_scheduled", |c| c.irq.scheduled.push((5, 1))),
        ("irq_wake_at", |c| c.irq.wake_at ^= 1),
        ("timer", |c| c.timer.tcount ^= 1),
        ("banks", |c| c.banks.m[0] ^= 1),
        ("skipped", |c| c.skipped += 1),
        ("device_io", |c| c.device_writes += 1),
        ("sdram_io", |c| c.sdram_reads += 1),
        ("periph", |c| c.enable_ringdma()),
        ("periph_log", |c| c.enable_access_log()),
        ("iop", |c| c.iop[5] ^= 1),
        ("isa_code", |c| c.isa_code.push(0x1234)),
        ("device", |c| {
            c.device.insert(0x3000_0010, 7);
        }),
        ("sdram", |c| c.sdram.write(crate::core::sdram::BASE + 16, 5)),
        ("sram", |c| c.sram[10] ^= 1),
    ];
    table.to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_compared_field_is_perturbed_and_detected_under_its_own_name() {
        let table = perturbations();
        let names: Vec<_> = table.iter().map(|(n, _)| *n).collect();
        assert_eq!(names, FIELDS, "the perturbation table follows FIELDS");
        for (name, perturb) in table {
            let (mut a, b) = (Core::new(&[]), Core::new(&[]));
            perturb(&mut a);
            let d = exact_diff(&a, &b).unwrap_or_else(|| panic!("{name} not detected"));
            assert_eq!(d.field, name, "{d}");
            let d = exact_diff(&b, &a).unwrap_or_else(|| panic!("{name} not detected"));
            assert_eq!(d.field, name, "{d}");
        }
    }

    #[test]
    fn two_perturbations_report_the_earlier_field_first() {
        let table = perturbations();
        let pick = |n: &str| table.iter().find(|(m, _)| *m == n).unwrap().1;
        let (mut a, b) = (Core::new(&[]), Core::new(&[]));
        pick("sram")(&mut a);
        pick("mode2")(&mut a);
        pick("iop")(&mut a);
        assert_eq!(exact_diff(&a, &b).map(|d| d.field), Some("mode2"));
    }
}
