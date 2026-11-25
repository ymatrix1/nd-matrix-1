//! Helpers the generated AOT modules call (`aot_emit.rs`) and the guard signature an interpreter
//! item inside a block must leave unchanged.
use super::decode_cache::ComputeOp;
use super::{Core, Fault};

/// MODE1 bits the rest of a block depends on: SIMD (both elements) and IRPTEN (arithmetic
/// exception interrupts).
const SIG_MODE1: u32 = 1 << 21 | 1 << 12;

/// State an interpreter item inside a block must leave unchanged for the rest of the block to
/// stay exact: the guard of [`Core::aot_ready`] re-checked piecewise (SIMD mode, enabled
/// arithmetic-exception interrupts, the interrupt controller's next event `due` (any IRPTL or
/// mask write that could raise one resets it), code, loop and call stacks, delayed branch).
/// Other MODE1 bits (rounding, saturation) are read again by every native batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct AotSig {
    /// MODE1 bits SIMD (21) and IRPTEN (12).
    mode: u32,
    /// Interrupt mask and the LIRPTL mask bits, which decide `arith_irq_enabled`.
    imask: u32,
    lmask: u32,
    due: u64,
    window_len: u32,
    idle: bool,
    code_gen: u64,
    loops: usize,
    calls: usize,
    pending: bool,
}

// Called only by the generated block module and the test oracle (`aot_state.rs`), so a build
// without `NORD_SHARC_AOT_HOT` sees them as unused.
#[allow(dead_code)]
impl Core {
    /// Exchanges the PEx/PEy views.
    #[inline(always)]
    pub(super) fn aot_swap_pe(&mut self) {
        self.swap_pe();
    }

    /// Runs one lowered compute field on the current element.
    #[inline(always)]
    pub(super) fn aot_op(&mut self, op: ComputeOp) {
        let ok = self.exec_compute_op(op);
        debug_assert!(ok, "AOT block holds an unmodelled compute op: {op:?}");
    }

    /// Reads the source of `Rdst = Rsrc`: value, float-extension tag and low byte.
    #[inline(always)]
    pub(super) fn aot_move_src(&self, src: u8) -> (u32, u32, u8) {
        let n = usize::from(src);
        (self.r[n], self.fx.tag[n], self.fx.lo[n])
    }

    /// Writes a captured register move; a move between data registers keeps the extension.
    #[inline(always)]
    pub(super) fn aot_move_dst(&mut self, dst: u8, m: (u32, u32, u8)) {
        let n = usize::from(dst);
        self.aot_write_ureg(dst, m.0);
        (self.fx.tag[n], self.fx.lo[n]) = (m.1, m.2);
    }

    /// `Rn = <shifter op> Rx BY imm` with the register-form opcode.
    #[inline(always)]
    pub(super) fn aot_shift(&mut self, op: u64, n: u8, x: u8, ry: u32) {
        let ok = self.shifter_op(op, usize::from(n), self.r[usize::from(x)], ry);
        debug_assert!(ok, "AOT block holds an unmodelled shift op {op:#x}");
    }

    /// The guarded state of the running block (see [`AotSig`]).
    #[inline(always)]
    pub(super) fn aot_sig(&self) -> AotSig {
        AotSig {
            mode: self.mode1 & SIG_MODE1,
            imask: self.imask,
            lmask: self.irq.lirptl >> 10 & 0x3FF,
            due: self.irq.due,
            window_len: self.irq.window_len,
            idle: self.irq.idle,
            code_gen: self.code_gen,
            loops: self.loops.len(),
            calls: self.call_stack.len(),
            pending: self.pending.is_some(),
        }
    }

    /// True when guarded state differs from `sig` (see [`AotSig`]), e.g. after an instruction of
    /// a whole-pass block wrote the interrupt controller or the mode bits.
    #[inline(always)]
    pub(super) fn aot_sig_changed(&self, sig: &AotSig) -> bool {
        self.aot_sig() != *sig
    }

    /// One interpreter step at `pc`, out of line (whole-pass blocks: instruction kinds without a
    /// native lowering).
    #[inline(never)]
    pub(super) fn aot_step(&mut self, pc: u32) -> Result<(), Fault> {
        use super::aot_prof::{aot_prof_on, aot_prof_step, tsc, GENERIC};
        self.pc = pc;
        if aot_prof_on() {
            let t = tsc();
            let r = self.step();
            aot_prof_step(pc, GENERIC, tsc() - t);
            return r;
        }
        self.step()
    }

    /// The tail of [`Core::step`] after a natively executed instruction: the pc (through the loop
    /// and delay-slot logic when one is active), the step count and the per-instruction latches.
    #[inline(always)]
    pub(super) fn aot_tail(&mut self, pc: u32, kind: crate::visa::Kind, next: u32) {
        if self.loops.is_empty()
            && self.pending.is_none()
            && self.cb_wrap.get() == 0
            && self.irq.window_len == 0
        {
            self.pc = next;
            self.steps += 1;
        } else {
            self.aot_tail_slow(pc, kind, next);
        }
    }

    /// [`Core::aot_tail`] with a loop, a pending delayed branch, a circular-buffer wrap or an open
    /// interrupt window: kept out of line so the hot path of every handler stays small.
    #[cold]
    #[inline(never)]
    fn aot_tail_slow(&mut self, pc: u32, kind: crate::visa::Kind, next: u32) {
        self.pc = if self.loops.is_empty() && self.pending.is_none() {
            next
        } else {
            self.finish_step(pc, kind, next)
        };
        self.steps += 1;
        if self.cb_wrap.get() != 0 {
            self.latch_cb_overflow();
        }
        if self.irq.window_len != 0 {
            self.close_window_if_left();
        }
    }

    /// [`Core::write_ureg`] with the data and DAG register files inline (the register code is a
    /// constant in a generated handler, so the match folds); system registers stay out of line.
    #[inline(always)]
    pub(super) fn aot_write_ureg(&mut self, code: u8, v: u32) {
        let n = (code & 15) as usize;
        match (code >> 4) & 7 {
            0 => {
                self.r[n] = v;
                self.fx.clear(n);
            }
            1 => self.i[n] = v,
            2 => self.m[n] = v,
            3 => self.l[n] = v,
            4 => {
                self.b[n] = v;
                self.i[n] = v;
            }
            5 => {
                self.s[n] = v;
                self.fx_y.clear(n);
            }
            _ => self.write_ureg(code, v),
        }
    }

    /// Runs the interpreter item at `pc` (one [`Core::step`], kept out of line so a block of
    /// thousands of items stays small) and returns true when the block must stop after it (see
    /// [`Core::aot_bail`]).
    #[inline(never)]
    pub(super) fn aot_item(&mut self, pc: u32, sig: &AotSig, next: u32) -> Result<bool, Fault> {
        self.pc = pc;
        self.step()?;
        Ok(self.aot_bail(sig, next))
    }

    /// After an interpreter item: true when the rest of the block must not run natively (the pc
    /// is not the next instruction, or guarded state changed). The core is then exactly where the
    /// interpreter would be.
    #[inline(always)]
    pub(super) fn aot_bail(&self, sig: &AotSig, next: u32) -> bool {
        self.pc != next || self.aot_sig() != *sig
    }
}

#[cfg(test)]
mod tests {
    use super::super::support::core_with;
    use super::*;

    fn simd_core() -> Core {
        let mut c = core_with(&[0x0001u16, 0x0001, 0x0001]);
        c.mode1 |= 1 << 21;
        c.irq.due = u64::MAX;
        c
    }

    #[test]
    fn bail_sees_pc_mode_and_code_changes() {
        let mut c = simd_core();
        let sig = c.aot_sig();
        let next = c.pc;
        assert!(!c.aot_bail(&sig, next));
        assert!(c.aot_bail(&sig, next + 1), "pc moved");
        c.mode1 ^= 1 << 21;
        assert!(c.aot_bail(&sig, next), "SIMD mode changed");
        c.mode1 ^= 1 << 21;
        c.mode1 ^= 1 << 13; // ALUSAT is re-read by each batch: no bail
        assert!(!c.aot_bail(&sig, next));
        c.mode1 ^= 1 << 13;
        let due = c.irq.due;
        c.irq.due = 5;
        assert!(
            c.aot_bail(&sig, next),
            "interrupt controller event time moved"
        );
        c.irq.due = due;
        c.code_gen += 1;
        assert!(c.aot_bail(&sig, next), "code changed");
    }

    #[test]
    fn item_runs_one_step_and_reports_no_bail_for_a_nop() {
        let mut c = simd_core();
        let sig = c.aot_sig();
        let (pc, steps) = (c.pc, c.steps);
        assert_eq!(c.aot_item(pc, &sig, pc + 1), Ok(false));
        assert_eq!((c.pc, c.steps), (pc + 1, steps + 1));
        assert_eq!(
            c.aot_item(pc + 1, &sig, pc + 5),
            Ok(true),
            "wrong next pc bails"
        );
    }

    /// Everything [`Core::aot_tail`] and the tail of [`Core::step`] may touch.
    fn tail_state(c: &Core) -> String {
        format!(
            "{:?}",
            (
                c.pc,
                c.steps,
                c.loops.len(),
                c.loops.last().map(|f| f.remaining),
                c.pending.map(|p| (p.slots, p.target)),
                c.cb_wrap.get(),
                c.irq.window_len,
                c.sys.stky[0],
            )
        )
    }

    /// Runs one NOP through `step` on one core and through `aot_tail` on a twin, after the same
    /// `setup`, and returns both end states.
    fn nop_both_ways(setup: impl Fn(&mut Core)) -> (Core, Core) {
        nop_both_ways_len(setup, 1)
    }

    /// [`nop_both_ways`] for a NOP that is `len` parcels long (an open window fetches in ISA order).
    fn nop_both_ways_len(setup: impl Fn(&mut Core), len: u32) -> (Core, Core) {
        let mut a = core_with(&[0x0001u16; 8]);
        let mut b = core_with(&[0x0001u16; 8]);
        setup(&mut a);
        setup(&mut b);
        a.step().unwrap();
        let (pc, kind) = (b.pc, b.fetch(b.pc).kind());
        b.aot_tail(pc, kind, pc + len);
        (a, b)
    }

    #[test]
    fn the_tail_matches_step_with_nothing_active() {
        let (a, b) = nop_both_ways(|_| {});
        assert_eq!(tail_state(&a), tail_state(&b));
        assert_eq!((b.pc, b.steps), (0x12_0001, 1));
    }

    #[test]
    fn the_tail_matches_step_at_a_loop_end_that_repeats_and_that_ends() {
        for remaining in [2, 1] {
            let (a, b) = nop_both_ways(|c| {
                c.loops.push(crate::core::types::LoopFrame {
                    start: 0x12_0000,
                    end: 0x12_0000,
                    remaining,
                    ..Default::default()
                });
            });
            assert_eq!(tail_state(&a), tail_state(&b), "remaining {remaining}");
            assert_eq!(b.loops.len(), usize::from(remaining > 1));
        }
    }

    #[test]
    fn the_tail_matches_step_with_a_pending_delayed_branch() {
        for (slots, call) in [(2, false), (1, false), (1, true)] {
            let (a, b) = nop_both_ways(|c| {
                c.pending = Some(crate::core::types::Pending {
                    slots,
                    target: 0x12_0005,
                    call,
                });
            });
            assert_eq!(tail_state(&a), tail_state(&b), "slots {slots} call {call}");
        }
        let (_, b) = nop_both_ways(|c| {
            c.pending = Some(crate::core::types::Pending {
                slots: 1,
                target: 0x12_0005,
                call: false,
            });
        });
        assert_eq!(b.pc, 0x12_0005, "the branch lands after its last slot");
    }

    #[test]
    fn the_tail_matches_step_with_a_circular_buffer_wrap_latched() {
        let (a, b) = nop_both_ways(|c| c.cb_wrap.set(1));
        assert_eq!(tail_state(&a), tail_state(&b));
        assert_eq!(b.cb_wrap.get(), 0, "the wrap is consumed");
        assert_ne!(b.sys.stky[0], 0, "and its sticky bit set");
    }

    #[test]
    fn the_tail_matches_step_with_an_open_interrupt_window() {
        for (lo, len) in [(0x12_0000, 1), (0x12_0000, 4)] {
            let (a, b) = nop_both_ways_len(
                |c| {
                    c.irq.window_lo = lo;
                    c.irq.window_len = len;
                },
                3,
            );
            assert_eq!(tail_state(&a), tail_state(&b), "window {len}");
            assert_eq!(b.irq.window_len, if len == 1 { 0 } else { len });
        }
    }
}
