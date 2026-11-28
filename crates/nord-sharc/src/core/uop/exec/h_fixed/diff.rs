//! The handler-level differential rig of the fixed-point family (recipe P8, design 6): one
//! instruction is lowered with [`FixedFamily`], its uop is run through the registered handler on a
//! core, the same instruction is run through `Core::step` on an identical core, and the two cores
//! are compared with `exact_diff` (every architectural and hidden field).
//!
//! OWNER: W2-FIXED, test support. Rules the rig enforces on every case:
//! * the handler either returns `Ok` and the core then equals the interpreter's after one step,
//!   or returns `Slow` and the core equals the core before the instruction (a bailing handler
//!   leaves the core untouched), after which `Core::step` finishes the instruction and the cores
//!   are compared again;
//! * `Slow` happens exactly when the interpreter raised an arithmetic-exception interrupt (the
//!   step pokes `irq.due` to 0; the core starts with `irq.due = u64::MAX`), so the bail rule has
//!   neither false alarms nor misses.
//!
//! Non-vacuous by construction (design P10): [`Case`] carries the data before and the core after,
//! so a test can count what actually happened (the state changed, an element ran), and the tests
//! of `diff_ops.rs` and `diff_wide.rs` assert minimums per kind and per opcode (the counters are
//! in `diff_sweep.rs`). `the_rig_catches_...` below show the comparison catches a handler that
//! computes the wrong thing, one that does not bail, one that bails for nothing and one that
//! writes before it bails.
use super::fixture::core_for;
use crate::core::types::Flags;
use crate::core::uop::config::UopConfig;
use crate::core::uop::exec::exit::materialize;
use crate::core::uop::lower::builder::BlockBuilder;
use crate::core::uop::lower::decode::decode_at;
use crate::core::uop::lower::fixed::FixedFamily;
use crate::core::uop::lower::{Family, LowerCtx};
use crate::core::uop::types::{Exit, Handler, UopCtx};
use crate::core::uop::verify::compare::exact_diff;
use crate::core::Core;

/// What one lowered instruction did on one state.
pub(super) struct Case {
    /// The kind of the uop the family emitted.
    pub kind: u16,
    /// Operand `a` of the uop (the compute word: opcode, registers).
    pub a: u32,
    /// What the handler returned.
    pub exit: Exit,
    /// The condition code of the uop.
    pub cond: u8,
    /// Whether PEx and PEy ran (condition true, and SIMD for PEy).
    pub ran: (bool, bool),
    /// Whether the interpreter raised an arithmetic-exception interrupt.
    pub raised: bool,
    /// The data before the instruction.
    pub before: Data,
    /// The core after it (handler, then `Core::step` when the handler bailed), which `exact_diff`
    /// found equal to the interpreter's.
    pub after: Core,
}

/// Everything a fixed-point compute can write: the registers and float extensions of both
/// elements, the multiplier registers, the flags, ASTAT extra bits and STKY.
#[derive(Clone, PartialEq)]
pub(super) struct Data {
    pub r: [u32; 16],
    pub s: [u32; 16],
    pub fx: ([u32; 16], [u8; 16]),
    pub fx_y: ([u32; 16], [u8; 16]),
    pub mr: [i128; 2],
    pub mr_y: [i128; 2],
    pub flags: (Flags, Flags),
    pub astat_extra: [u32; 2],
    pub stky: [u32; 2],
}

impl Data {
    pub fn of(c: &Core) -> Data {
        Data {
            r: c.r,
            s: c.s,
            fx: (c.fx.tag, c.fx.lo),
            fx_y: (c.fx_y.tag, c.fx_y.lo),
            mr: c.mr,
            mr_y: c.mr_y,
            flags: (c.flags, c.flags_y),
            astat_extra: c.sys.astat_extra,
            stky: c.sys.stky,
        }
    }
}

impl Case {
    /// Whether the instruction changed any of the data an op can write (the pc and the step
    /// count do not count: they change for every instruction).
    pub fn changed(&self) -> bool {
        Data::of(&self.after) != self.before
    }
}

/// How a case runs the handler.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Mode {
    /// The registered handlers: `Ok` and equal to the interpreter, or `Slow` with the core
    /// untouched and the interpreter having raised an exception.
    Real,
    /// The handlers without the bail: always `Ok`, and equal to the interpreter in everything but
    /// the raise itself (STKY, the interrupt latches and `irq.due`, which the handler does not
    /// touch). That proves the arithmetic of the cases the real handler bails on.
    Forced,
}

/// Runs `program` (one instruction) once on the state of `seed`: `Ok(None)` when the family
/// declines it, `Ok(Some(case))` when handler and interpreter agree, `Err(text)` with the first
/// differing field otherwise.
pub(super) fn run_case(
    table: &[Handler],
    mode: Mode,
    program: &[u16],
    seed: u64,
    simd: bool,
) -> Result<Option<Case>, String> {
    run_case_with(table, mode, program, seed, simd, &|_| ())
}

/// [`run_case`] on the state of `seed` after `setup` changed it (the same way for the cores of the
/// handler, the interpreter and, when the handler bails, the untouched copy), for a test that
/// needs one exact state.
pub(super) fn run_case_with(
    table: &[Handler],
    mode: Mode,
    program: &[u16],
    seed: u64,
    simd: bool,
    setup: &dyn Fn(&mut Core),
) -> Result<Option<Case>, String> {
    let build = || {
        let mut c = core_for(program, seed, simd);
        setup(&mut c);
        c
    };
    let mut dut = build();
    let at = format!("program {program:04x?} seed {seed} simd {simd} {mode:?}");
    let pc = dut.pc;
    let insn = decode_at(&mut dut, pc);
    let cfg = UopConfig::default();
    let cx = LowerCtx {
        cfg: &cfg,
        stop_pc: None,
        simd,
        isa: false,
    };
    let mut block = BlockBuilder::new(pc, simd, false);
    if !FixedFamily::lower(&cx, &insn, &mut block) {
        return Ok(None);
    }
    if block.len() != 1 || block.steps() != 1 {
        return Err(format!(
            "{at}: {} uops, {} steps",
            block.len(),
            block.steps()
        ));
    }
    let u = block.uops()[0];
    // built only for a lowered word: the sweeps run many words the family declines
    let mut oracle = build();
    let before = Data::of(&dut);
    let cond = (u.b & 31) as u8;
    let ran = (
        dut.cond(u64::from(cond)),
        simd && dut.cond_for(true, u64::from(cond)),
    );
    oracle
        .step()
        .map_err(|f| format!("{at}: the interpreter faulted on a lowered word: {f:?}"))?;
    let steps = dut.steps;
    let mut ctx = UopCtx::new(&mut dut as *mut Core, steps, 4);
    // SAFETY: `ctx.core` is the only live reference to `dut` during the call; the handler is the
    // one registered for the kind of `u`.
    let raw = unsafe { table[usize::from(u.h)](&mut ctx, &u) };
    let exit = Exit::from_u32(raw).ok_or_else(|| format!("{at}: exit code {raw}"))?;
    let raised = oracle.irq.due != u64::MAX;
    match (exit, mode) {
        (Exit::Ok, Mode::Real) if raised => {
            return Err(format!(
                "{at}: the handler ran but the interpreter raised an exception"
            ));
        }
        (Exit::Ok, _) => {
            ctx.done += u32::from(u.steps);
            materialize(&mut dut, &ctx, insn.next_pc);
            if mode == Mode::Forced {
                // the raise is the one thing the forced handler does not do: take it from the
                // interpreter so that everything else is compared
                dut.sys.stky = oracle.sys.stky;
                (dut.irq.irptl, dut.irq.lirptl, dut.irq.due) =
                    (oracle.irq.irptl, oracle.irq.lirptl, oracle.irq.due);
            }
        }
        (Exit::Slow, Mode::Real) => {
            if !raised {
                return Err(format!(
                    "{at}: the handler bailed but the interpreter raised nothing"
                ));
            }
            // the core must be exactly what it was before the call: a fresh copy of the state
            if let Some(d) = exact_diff(&dut, &build()) {
                return Err(format!("{at}: a bailing handler changed the core: {d}"));
            }
            dut.step()
                .map_err(|f| format!("{at}: step after the bail: {f:?}"))?;
        }
        (other, _) => return Err(format!("{at}: unexpected exit {other:?}")),
    }
    if let Some(d) = exact_diff(&dut, &oracle) {
        return Err(format!("{at}: kind {:#x}, exit {exit:?}: {d}", u.h));
    }
    Ok(Some(Case {
        kind: u.h,
        a: u.a,
        exit,
        cond,
        ran,
        raised,
        before,
        after: dut,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::uop::exec::h_fixed::fixture::t2a;
    use crate::core::uop::hot::UopHot;
    use crate::core::uop::types::{bail_handler, Uop};

    /// Writes R5 and then bails: a handler that breaks the rule "nothing written before a bail".
    unsafe extern "C" fn dirty_bail(ctx: *mut UopCtx, _: *const Uop) -> u32 {
        // SAFETY: `ctx.core` is the one live reference to the core during the call (the rig).
        unsafe { (*(*ctx).core).r[5] ^= 1 };
        Exit::Slow as u32
    }

    #[test]
    fn the_rig_catches_a_missing_bail_a_needless_bail_and_a_dirty_bail() {
        // R3 = R1 + R2 (IF TRUE) with the registers set to overflow, or not to
        let word = t2a(31, 0x1312);
        let over = |c: &mut Core| (c.r[1], c.r[2]) = (0x7FFF_FFFF, 1);
        let calm = |c: &mut Core| (c.r[1], c.r[2]) = (1, 2);
        let real = UopHot::handler_table().into_vec();
        assert!(run_case_with(&real, Mode::Real, &word, 1, false, &over).is_ok());
        assert!(run_case_with(&real, Mode::Real, &word, 1, false, &calm).is_ok());
        // the handler that never bails runs an instruction the interpreter raised an exception on
        let never = super::super::forced_table();
        let e = run_case_with(&never, Mode::Real, &word, 1, false, &over)
            .err()
            .unwrap();
        assert!(e.contains("raised an exception"), "{e}");
        // the handler that always bails does so on an instruction that raised nothing
        let mut always = real.clone();
        always[0x201] = bail_handler as Handler;
        let e = run_case_with(&always, Mode::Real, &word, 1, false, &calm)
            .err()
            .unwrap();
        assert!(e.contains("raised nothing"), "{e}");
        // the handler that writes a register before it bails changed the core
        let mut dirty = real.clone();
        dirty[0x201] = dirty_bail as Handler;
        let e = run_case_with(&dirty, Mode::Real, &word, 1, false, &over)
            .err()
            .unwrap();
        assert!(e.contains("changed the core"), "{e}");
    }

    #[test]
    fn the_rig_catches_a_handler_that_computes_the_wrong_thing() {
        // R3 = R1 + R2 (IF TRUE), with the ADD slot holding the SUB handler
        let table = UopHot::handler_table().into_vec();
        let word = t2a(31, 0x1312);
        let mut wrong = table.clone();
        wrong[0x201] = table[0x202];
        let mut differ = 0;
        for seed in 1..20 {
            let ok = run_case(&table, Mode::Real, &word, seed, false).unwrap();
            assert!(
                ok.is_some_and(|c| c.kind == 0x201),
                "the real handler agrees"
            );
            differ += usize::from(run_case(&wrong, Mode::Real, &word, seed, false).is_err());
        }
        assert!(
            differ >= 15,
            "the wrong handler differed in {differ} of 19 states"
        );
        // a forced run of a bailing case still has to equal the interpreter
        let forced = super::super::forced_table();
        assert!((1..40).all(|s| run_case(&forced, Mode::Forced, &word, s, true).is_ok()));
        assert!(
            run_case(&table, Mode::Real, &[0x0001], 5, false)
                .unwrap()
                .is_none(),
            "NOP is declined"
        );
    }

    #[test]
    fn a_condition_that_fails_changes_no_data_and_the_comparison_sees_a_change() {
        let table = UopHot::handler_table().into_vec();
        let (mut quiet, mut moved) = (0, 0);
        for seed in 1..60 {
            // R3 = R1 + R2 IF EQ on both elements
            let c = run_case(&table, Mode::Real, &t2a(0, 0x1312), seed, true)
                .unwrap()
                .unwrap();
            if !(c.ran.0 || c.ran.1) {
                assert!(!c.changed(), "seed {seed}: a failed condition changed data");
            }
            quiet += usize::from(!c.changed());
            moved += usize::from(c.changed());
        }
        assert!(
            quiet >= 5 && moved >= 5,
            "both outcomes occur: quiet {quiet}, moved {moved}"
        );
    }
}
