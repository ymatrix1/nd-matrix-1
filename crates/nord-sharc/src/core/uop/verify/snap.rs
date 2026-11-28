//! Snapshot and restore of every field `exact_diff` compares, except SRAM, IOP and the peripheral
//! models (`periph`, `periph_log`).
//!
//! OWNER: W1-VERIFY. A snapshot allocates (it is taken by tools, shadow mode and translation
//! validation, never on the audio path): a few KB of registers and stacks, plus a copy of the
//! sparse device store, of the SDRAM window when it was written (up to 1 MiB) and of the ISA word
//! image when one is loaded. SRAM and IOP are not captured: shadow mode logs stores in an undo log
//! instead (design 4.5 rule 4 and 6.6), and tests that restore after a memory write undo it
//! themselves. The device store, SDRAM and ISA words are captured because the shadow check
//! restores into a fresh core and then compares everything, and nothing logs their stores. The
//! peripheral models cannot be captured (most are neither `Clone` nor readable), which is why
//! shadow mode, translation validation and `UopChecked` skip cores that have them (design risk 6).
//!
//! Its tests apply every perturbation of `compare/perturb.rs` and every row of `compare/rows.rs`
//! except the two memory images and the peripheral names, and check that `restore` brings back a
//! state `exact_diff` calls identical, so a field added to the comparator without a snapshot slot
//! fails here.
use crate::core::addr_hash::DeviceMap;
use crate::core::interrupts::Serviced;
use crate::core::types::{Flags, LoopFrame, Pending};
use crate::core::Core;

/// Everything `exact_diff` compares except SRAM, IOP and the peripheral models.
#[derive(Clone, Debug)]
pub struct CoreSnap {
    pc: u32,
    steps: u64,
    regs: [[u32; 16]; 6],
    mr: [i128; 2],
    mr_y: [i128; 2],
    fx: ([u32; 16], [u8; 16]),
    fx_y: ([u32; 16], [u8; 16]),
    flags: Flags,
    flags_y: Flags,
    modes: [u32; 4],
    ustat: [u32; 4],
    stky: [u32; 2],
    astat_extra: [u32; 2],
    sys_status: Vec<[u32; 3]>,
    sys_bits: (bool, u32, u16, u64),
    cb_wrap: u8,
    call_stack: Vec<u32>,
    loops: Vec<LoopFrame>,
    pending: Option<Pending>,
    irq_words: [u32; 6],
    irq_wide: [u64; 4],
    irq_serviced: Vec<Serviced>,
    irq_scheduled: Vec<(u64, u8)>,
    timer: (u32, u32, bool, u64, u32, u64),
    banks: [[u32; 16]; 6],
    counters: ([u64; 5], Option<u32>),
    device: DeviceMap,
    sdram: Vec<u32>,
    isa_code: Vec<u16>,
}

impl CoreSnap {
    /// Captures the state of `c`.
    pub fn capture(c: &Core) -> Self {
        let t = &c.timer;
        CoreSnap {
            pc: c.pc,
            steps: c.steps,
            regs: [c.r, c.s, c.i, c.m, c.l, c.b],
            mr: c.mr,
            mr_y: c.mr_y,
            fx: (c.fx.tag, c.fx.lo),
            fx_y: (c.fx_y.tag, c.fx_y.lo),
            flags: c.flags,
            flags_y: c.flags_y,
            modes: [c.mode1, c.mode2, c.imask, c.mmask],
            ustat: c.ustat,
            stky: c.sys.stky,
            astat_extra: c.sys.astat_extra,
            sys_status: c.sys.status.clone(),
            sys_bits: (
                c.sys.pc_overflow,
                c.sys.flags_reg,
                c.sys.flag_pins,
                c.sys.px,
            ),
            cb_wrap: c.cb_wrap.get(),
            call_stack: c.call_stack.clone(),
            loops: c.loops.clone(),
            pending: c.pending,
            irq_words: [
                c.irq.irptl,
                c.irq.lirptl,
                u32::from(c.irq.idle),
                c.irq.ivt_base,
                c.irq.window_lo,
                c.irq.window_len,
            ],
            irq_wide: [c.irq.stall, c.irq.due, c.irq.taken, c.irq.wake_at],
            irq_serviced: c.irq.serviced.clone(),
            irq_scheduled: c.irq.scheduled.clone(),
            timer: (
                t.tcount,
                t.tperiod,
                t.running,
                t.base_cycle,
                t.base_count,
                t.seen,
            ),
            banks: {
                let b = &c.banks;
                [b.r, b.s, b.i, b.m, b.l, b.b]
            },
            counters: (
                [
                    c.skipped,
                    c.device_reads,
                    c.device_writes,
                    c.sdram_reads,
                    c.sdram_writes,
                ],
                c.first_skipped,
            ),
            device: c.device.clone(),
            sdram: c.sdram.mem.clone(),
            isa_code: c.isa_code.clone(),
        }
    }

    /// Writes the captured state back into `c`.
    pub fn restore(&self, c: &mut Core) {
        c.pc = self.pc;
        c.steps = self.steps;
        [c.r, c.s, c.i, c.m, c.l, c.b] = self.regs;
        c.mr = self.mr;
        c.mr_y = self.mr_y;
        (c.fx.tag, c.fx.lo) = self.fx;
        (c.fx_y.tag, c.fx_y.lo) = self.fx_y;
        c.flags = self.flags;
        c.flags_y = self.flags_y;
        [c.mode1, c.mode2, c.imask, c.mmask] = self.modes;
        c.ustat = self.ustat;
        c.sys.stky = self.stky;
        c.sys.astat_extra = self.astat_extra;
        c.sys.status.clone_from(&self.sys_status);
        (
            c.sys.pc_overflow,
            c.sys.flags_reg,
            c.sys.flag_pins,
            c.sys.px,
        ) = self.sys_bits;
        c.cb_wrap.set(self.cb_wrap);
        c.call_stack.clone_from(&self.call_stack);
        c.loops.clone_from(&self.loops);
        c.pending = self.pending;
        let w = &self.irq_words;
        (c.irq.irptl, c.irq.lirptl, c.irq.idle) = (w[0], w[1], w[2] != 0);
        (c.irq.ivt_base, c.irq.window_lo, c.irq.window_len) = (w[3], w[4], w[5]);
        let v = &self.irq_wide;
        (c.irq.stall, c.irq.due, c.irq.taken, c.irq.wake_at) = (v[0], v[1], v[2], v[3]);
        c.irq.serviced.clone_from(&self.irq_serviced);
        c.irq.scheduled.clone_from(&self.irq_scheduled);
        let t = &mut c.timer;
        (
            t.tcount,
            t.tperiod,
            t.running,
            t.base_cycle,
            t.base_count,
            t.seen,
        ) = self.timer;
        let b = &mut c.banks;
        [b.r, b.s, b.i, b.m, b.l, b.b] = self.banks;
        let (n, first) = self.counters;
        (c.skipped, c.device_reads, c.device_writes) = (n[0], n[1], n[2]);
        (c.sdram_reads, c.sdram_writes) = (n[3], n[4]);
        c.first_skipped = first;
        c.device.clone_from(&self.device);
        c.sdram.mem.clone_from(&self.sdram);
        c.isa_code.clone_from(&self.isa_code);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::sdram;
    use crate::core::uop::verify::compare::{
        exact_diff, nested_rows, perturbations, prepared_filled,
    };
    use crate::core::uop::verify::gen::{
        fill_astat_extra, fill_fifo, fill_stacks, fill_state, fill_sticky, Rng,
    };

    /// Whether a snapshot holds the field `exact_diff` reports under `name`: not the two memory
    /// images the caller undoes, and not the peripheral models.
    fn snapshotted(name: &str) -> bool {
        !matches!(name, "sram" | "iop" | "periph" | "periph_log")
    }

    /// A core with every kind of state the snapshot must carry, from one seed.
    fn filled(seed: u64) -> Core {
        let mut c = Core::new(&[]);
        let mut rng = Rng::new(seed);
        fill_state(&mut c, &mut rng, seed.is_multiple_of(2));
        fill_sticky(&mut c, &mut rng);
        fill_astat_extra(&mut c, &mut rng);
        fill_fifo(&mut c, &mut rng);
        fill_stacks(&mut c, &mut rng, 4, 3);
        for k in 0..5 {
            c.dm_write(0x3000_0000 + 4 * k, rng.next_u32()).unwrap();
            c.dm_write(sdram::BASE + rng.below(0x100), rng.next_u32())
                .unwrap();
            c.dm_read(0x3000_0100 + k).unwrap();
        }
        c.isa_code.extend_from_slice(&[1, 2, 3, 4, 5, 6]);
        c.banks.r[2] = rng.next_u32();
        c.cb_wrap.set(2);
        c.pc = 0x12_0040;
        c.steps = u64::from(rng.next_u32());
        c
    }

    #[test]
    fn restore_undoes_every_compared_field_except_the_memory_images() {
        let (mut a, b) = (Core::new(&[]), Core::new(&[]));
        let snap = CoreSnap::capture(&a);
        let mut applied = Vec::new();
        for (name, perturb) in perturbations() {
            if !snapshotted(name) {
                continue;
            }
            perturb(&mut a);
            applied.push(name);
        }
        for covered in [
            "device",
            "sdram",
            "isa_code",
            "banks",
            "f40_y",
            "fx_x",
            "fx_y",
            "irq_serviced",
        ] {
            assert!(applied.contains(&covered), "{covered} is perturbed");
        }
        assert!(exact_diff(&a, &b).is_some());
        snap.restore(&mut a);
        assert_eq!(
            exact_diff(&a, &b),
            None,
            "after {} perturbations",
            applied.len()
        );
    }

    #[test]
    fn each_perturbation_alone_is_undone_by_restore() {
        for (name, perturb) in perturbations() {
            if !snapshotted(name) {
                continue;
            }
            let (mut a, b) = (Core::new(&[]), Core::new(&[]));
            let snap = CoreSnap::capture(&a);
            perturb(&mut a);
            assert!(exact_diff(&a, &b).is_some(), "{name} changes the core");
            snap.restore(&mut a);
            assert_eq!(exact_diff(&a, &b), None, "{name} survived restore");
        }
    }

    #[test]
    fn every_nested_field_has_a_snapshot_slot() {
        // the coverage rows change one sub-field each (timer.seen, banks.l, loop frame fields, ...)
        let rows = nested_rows();
        assert!(rows.len() > 90);
        for seed in [0, 3, 8] {
            for (s, f, name, perturb) in rows.iter().filter(|r| snapshotted(r.2)) {
                let (mut a, b) = (prepared_filled(seed), prepared_filled(seed));
                let snap = CoreSnap::capture(&a);
                perturb(&mut a);
                assert_eq!(exact_diff(&a, &b).map(|d| d.field), Some(*name), "{s}.{f}");
                snap.restore(&mut a);
                assert_eq!(
                    exact_diff(&a, &b),
                    None,
                    "{s}.{f} survived restore (seed {seed})"
                );
            }
        }
    }

    #[test]
    fn capture_then_restore_on_a_changed_core_is_the_identity() {
        let mut a = Core::new(&[]);
        a.pc = 0x12_0040;
        a.r[5] = 0xDEAD_BEEF;
        a.call_stack.push(0x12_0044);
        let snap = CoreSnap::capture(&a);
        let mut b = Core::new(&[]);
        snap.restore(&mut b);
        assert_eq!(exact_diff(&a, &b), None);
        assert_eq!(
            (b.pc, b.r[5], b.call_stack.clone()),
            (0x12_0040, 0xDEAD_BEEF, vec![0x12_0044])
        );
    }

    #[test]
    fn a_fully_filled_state_survives_capture_and_restore_into_a_fresh_core() {
        for seed in 1..25 {
            let a = filled(seed);
            assert!(!a.device.is_empty() && !a.sdram.mem.is_empty());
            let mut b = Core::new(&[]);
            CoreSnap::capture(&a).restore(&mut b);
            assert_eq!(exact_diff(&a, &b), None, "seed {seed}");
            // and a second restore, from a snapshot of the restored core, is still the identity
            let mut c = Core::new(&[]);
            CoreSnap::capture(&b).restore(&mut c);
            assert_eq!(exact_diff(&a, &c), None, "seed {seed}, second generation");
        }
    }

    #[test]
    fn a_snapshot_is_a_copy_and_restore_removes_what_it_did_not_have() {
        let empty = CoreSnap::capture(&Core::new(&[]));
        let mut c = filled(9);
        let full = CoreSnap::capture(&c);
        empty.restore(&mut c);
        assert_eq!(
            exact_diff(&c, &Core::new(&[])),
            None,
            "device, SDRAM and ISA words are gone"
        );
        assert!(c.device.is_empty() && c.sdram.mem.is_empty() && c.isa_code.is_empty());
        full.restore(&mut c);
        assert_eq!(
            exact_diff(&c, &filled(9)),
            None,
            "the first snapshot was not disturbed"
        );
    }

    #[test]
    fn sram_and_iop_are_not_part_of_the_snapshot() {
        let mut a = Core::new(&[]);
        let snap = CoreSnap::capture(&a);
        a.sram[10] ^= 1;
        a.iop[3] = 7;
        snap.restore(&mut a);
        assert_eq!(
            (a.sram[10], a.iop[3]),
            (1, 7),
            "memory writes are the caller's to undo"
        );
    }
}
