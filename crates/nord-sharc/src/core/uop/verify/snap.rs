//! Snapshot and restore of every field `exact_diff` compares, except the memory images.
//!
//! OWNER: W1-VERIFY. A snapshot is a few KB and allocates (it is taken by tools, shadow mode and
//! translation validation, never on the audio path). SRAM and IOP are not captured: shadow mode
//! logs stores in an undo log instead (design 4.5 rule 4 and 6.6), and tests that restore after a
//! memory write undo it themselves.
//!
//! Skeleton: complete. Its test applies every perturbation of `compare.rs` except the two memory
//! ones and checks that `restore` brings back a state `exact_diff` calls identical, so a field
//! added to the comparator without a snapshot slot fails here.
use crate::core::interrupts::Serviced;
use crate::core::types::{Flags, LoopFrame, Pending};
use crate::core::Core;

/// Everything `exact_diff` compares except SRAM and IOP.
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::uop::verify::compare::{exact_diff, perturbations};

    #[test]
    fn restore_undoes_every_compared_field_except_the_memory_images() {
        let (mut a, b) = (Core::new(&[]), Core::new(&[]));
        let snap = CoreSnap::capture(&a);
        let mut applied = 0;
        for (name, perturb) in perturbations() {
            if name == "sram" || name == "iop" {
                continue;
            }
            perturb(&mut a);
            applied += 1;
        }
        assert!(exact_diff(&a, &b).is_some());
        snap.restore(&mut a);
        assert_eq!(exact_diff(&a, &b), None, "after {applied} perturbations");
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
}
