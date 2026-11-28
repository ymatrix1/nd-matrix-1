//! Event-aware blocks (stage D1): a timer or interrupt-controller event due inside a generated
//! block no longer refuses the block. The block checks `steps >= irq.due` at each item boundary
//! (v1 required the whole block to fit before `due`); when an event is due, the item runs on the
//! interpreter, whose `step` takes the event exactly where it would, and the block carries on
//! natively unless an interrupt was taken (then it exits with the interpreter's exact state).
//!
//! Exactness: the interpreter handles the event, so the state at the split is the interpreter's by
//! construction; the native code between events is v1's. `due` is read live, so a block no longer
//! needs `due` in its guard signature ([`Core::aot_sig_ev`]).
use crate::core::aot_glue::AotSig;
use crate::core::{Core, Fault};

/// Returned by an event item that must end the block (an interrupt was taken, the pc is not the
/// next instruction). `aot_hot.rs` treats it as an early exit, never as a fault: real pcs are
/// never `u32::MAX`.
pub(in crate::core) const SPLIT: Fault = Fault::Unimplemented {
    pc: u32::MAX,
    parcels: [0xA07E; 3],
};

// Called only by generated blocks (`aot_emit.rs`): a build without a module sees them as unused.
#[allow(dead_code)]
impl Core {
    /// One interpreter step at `pc` because an event is due at this boundary. `Err(SPLIT)` when an
    /// interrupt was taken or the pc is not `next`.
    #[cold]
    #[inline(never)]
    pub(in crate::core) fn aot_ev_step(&mut self, pc: u32, next: u32) -> Result<(), Fault> {
        let taken = self.irq.taken;
        self.pc = pc;
        self.step()?;
        if self.irq.taken != taken || self.pc != next {
            return Err(SPLIT);
        }
        Ok(())
    }

    /// `aot_step` for an event-aware block: the interpreter step the item always was, plus the
    /// split when it took an event.
    #[inline(always)]
    pub(in crate::core) fn aot_step_ev(&mut self, pc: u32, next: u32) -> Result<(), Fault> {
        if self.steps >= self.irq.due {
            return self.aot_ev_step(pc, next);
        }
        self.aot_step(pc)
    }

    /// `aot_item` for an event-aware block: true when the block must stop after it.
    #[inline(never)]
    pub(in crate::core) fn aot_item_ev(
        &mut self,
        pc: u32,
        sig: &AotSig,
        next: u32,
    ) -> Result<bool, Fault> {
        let taken = self.irq.taken;
        self.pc = pc;
        self.step()?;
        Ok(self.irq.taken != taken || self.pc != next || self.aot_sig_changed_ev(sig))
    }

    /// `n` interpreter steps from `pc` for a native batch an event falls into. `Err(SPLIT)` when an
    /// interrupt was taken or the pc after them is not `next`.
    #[cold]
    #[inline(never)]
    pub(in crate::core) fn aot_ev_span(&mut self, pc: u32, n: u32, next: u32) -> Result<(), Fault> {
        let taken = self.irq.taken;
        self.pc = pc;
        for _ in 0..n {
            self.step()?;
        }
        if self.irq.taken != taken || self.pc != next {
            return Err(SPLIT);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::support::core_with;

    fn core() -> Core {
        core_with(&[0x0001; 8]) // eight 16-bit NOPs
    }

    fn with_interrupt() -> Core {
        let mut c = core();
        c.mode1 |= 1 << 12; // IRPTEN
        c.imask |= 1 << 11;
        c.schedule_interrupt(0, 11);
        c
    }

    #[test]
    fn an_event_step_is_the_interpreter_step_when_no_interrupt_is_taken() {
        let (mut a, mut b) = (core(), core());
        (a.irq.due, b.irq.due) = (0, 0);
        let pc = a.pc;
        assert_eq!(a.aot_ev_step(pc, pc + 1), Ok(()));
        b.step().unwrap();
        assert_eq!((a.pc, a.steps, a.irq.due), (b.pc, b.steps, b.irq.due));
        assert_eq!(a.aot_step_ev(a.pc, a.pc + 1), Ok(()), "event due or not");
    }

    #[test]
    fn a_span_with_an_event_inside_equals_the_same_interpreter_steps() {
        let (mut a, mut b) = (core(), core());
        a.irq.due = a.steps + 2;
        b.irq.due = b.steps + 2;
        let pc = a.pc;
        assert_eq!(a.aot_ev_span(pc, 4, pc + 4), Ok(()));
        b.run(4);
        assert_eq!((a.pc, a.steps, a.irq.due), (b.pc, b.steps, b.irq.due));
    }

    #[test]
    fn an_interrupt_taken_by_the_event_splits_the_block_with_the_interpreter_state() {
        let mut c = with_interrupt();
        let (pc, taken) = (c.pc, c.irq.taken);
        assert_eq!(c.aot_ev_step(pc, pc + 1), Err(SPLIT));
        assert_eq!(c.irq.taken, taken + 1);
        let mut i = with_interrupt();
        i.step().unwrap();
        assert_eq!(
            (c.pc, c.steps),
            (i.pc, i.steps),
            "state is the interpreter's"
        );
        let mut s = with_interrupt();
        let pc = s.pc;
        assert_eq!(s.aot_ev_span(pc, 3, pc + 3), Err(SPLIT));
    }

    #[test]
    fn an_interpreter_item_stops_the_block_on_an_event_and_not_otherwise() {
        let mut c = core();
        c.irq.due = u64::MAX;
        let sig = c.aot_sig_ev();
        let pc = c.pc;
        assert_eq!(c.aot_item_ev(pc, &sig, pc + 1), Ok(false));
        let mut c = with_interrupt();
        let sig = c.aot_sig_ev();
        let pc = c.pc;
        assert_eq!(c.aot_item_ev(pc, &sig, pc + 1), Ok(true));
    }
}
