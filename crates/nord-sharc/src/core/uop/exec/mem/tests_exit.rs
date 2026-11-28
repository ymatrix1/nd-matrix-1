//! Tests of the exits of [`super::Mem`]: the uop boundary the slow path materializes, the order of
//! the accesses, and `finish` (`Jump` after a moved deadline, `Smc` after a store into code).
//!
//! OWNER: W2-MOVE, test build only.
use super::*;
use crate::core::support::core_with;
use crate::core::uop::types::UopKind;
use crate::core::uop::verify::compare::exact_diff;

/// The slow path of [`Mem`] materializes `pc` and `steps` before it looks at memory, so a
/// watch hit and a read-watch hit carry what the interpreter would log, and the accesses
/// reach the models in program order.
#[test]
fn slow_accesses_see_the_uop_boundary_and_keep_their_order() {
    let (mut a, mut b) = (core_with(&[0x0001]), core_with(&[0x0001]));
    for c in [&mut a, &mut b] {
        c.enable_access_log();
        c.watch = 0x3_E000;
        c.watch_len = 8;
        c.rwatch = 0x3_E000;
        c.rwatch_len = 8;
        c.rwatch_on = true;
    }
    // the interpreter's view: `pc`/`steps` as the instruction sees them
    b.pc = 0x12_0007;
    b.steps = 42;
    let seq: [(bool, u32, u32); 5] = [
        (true, 0x3_E001, 11),
        (false, 0x3_E002, 0),
        (true, 0x3_E000, 12),
        (false, 0x3_E001, 0),
        (true, 0x9_0001, 13), // SRAM in between: not logged by the census
    ];
    for (write, addr, v) in seq {
        if write {
            b.dm_write(addr, v).unwrap();
        } else {
            b.dm_read(addr).unwrap();
        }
    }
    // the handler's view: the core still sits at block entry, the context knows the rest
    a.pc = 0x12_0000;
    a.steps = 40;
    let mut ctx = UopCtx::new(&mut a as *mut Core, 40, 100);
    ctx.done = 2;
    let u = Uop::new(UopKind(UopKind::MOVE_BASE), 1, 0x12_0007, 0, 0);
    let mut m = unsafe { Mem::new(&mut ctx, &u) };
    for (write, addr, v) in seq {
        if write {
            assert_eq!(m.wr(addr, v), Ok(()));
        } else {
            assert!(m.rd(addr).is_ok());
        }
    }
    assert_eq!(m.finish(true), Exit::Ok as u32, "no deadline moved");
    assert_eq!(a.watch_hits, b.watch_hits);
    assert_eq!(a.watch_addrs, b.watch_addrs);
    assert_eq!(a.rwatch_hits, b.rwatch_hits);
    assert_eq!(a.watch_hits.len(), 2, "the two stores to the watched words");
    assert_eq!(a.watch_hits[0], (42, 0x12_0007, 11));
    // the census saw the same accesses in the same order; only pc and steps differ by design
    a.pc = b.pc;
    a.steps = b.steps;
    assert_eq!(exact_diff(&a, &b), None);
}

#[test]
fn finish_reports_a_moved_deadline_and_a_store_into_code() {
    let mut c = core_with(&[0x0001]);
    c.enable_sc57x(1);
    c.irq.due = u64::MAX;
    c.sc57x().unwrap().raise_source(69);
    let mut ctx = UopCtx::new(&mut c as *mut Core, 0, 100);
    // a 3-parcel instruction: the pc after it is u.pc + 3
    let u = Uop::new(UopKind(UopKind::MOVE_BASE), 1, 0x12_0010, 3 << LEN_SHIFT, 0);
    let mut m = unsafe { Mem::new(&mut ctx, &u) };
    // a store to a modelled register drains the queued SEC interrupt: `irq.due` drops to 0
    assert_eq!(m.wr(0x3108_900C, 69), Ok(()));
    assert_eq!(m.finish(true), Exit::Jump as u32);
    assert_eq!(ctx.next_pc, 0x12_0013);
    assert_eq!(c.irq.due, 0, "the event is due now");
    // a load that changes nothing needs no exit
    let mut ctx = UopCtx::new(&mut c as *mut Core, 0, 100);
    let mut m = unsafe { Mem::new(&mut ctx, &u) };
    assert!(m.rd(0x70_0000).is_ok());
    assert_eq!(m.finish(false), Exit::Ok as u32);
    // a store that hit a code page: Smc, and nothing else needed
    c.uop_run.flags = RF_SMC;
    let mut ctx = UopCtx::new(&mut c as *mut Core, 0, 100);
    let mut m = unsafe { Mem::new(&mut ctx, &u) };
    assert_eq!(m.finish(true), Exit::Smc as u32);
    assert_eq!(m.finish(false), Exit::Ok as u32, "a load never looks at it");
}
