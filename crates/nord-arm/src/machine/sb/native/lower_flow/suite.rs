//! OWNER: W2-NMEM. Trace generators and comparison suites of the native control-flow tests
//! (`cfg(test)`): the loop traces, the exit-form traces and the kill suite of the flow mutants.

use super::super::lower_mem::harness::compare::{run_case, Case};
use super::super::lower_mem::harness::oracle::Tally;
use super::super::lower_mem::harness::{
    add_imm, compile_test, page_addr, random_state, HOOKED_R, RAM0, RAM1, RAM2,
};
use super::super::rt::Jit;
use crate::machine::sb::exec_mem::{F_P, F_U, F_WB};
use crate::machine::sb::testkit::Rng;
use crate::machine::sb::uop::fused::fuse_pair;
use crate::machine::sb::uop::{k, Exit, ExitKind, Sb, Uop};

/// A loop trace and the registers a case must set so the loop runs: `r0..=r3` pointers that
/// reach the end of a page after a few iterations, `r5` the counter.
pub(crate) struct LoopTrace {
    pub sb: Sb,
    /// Counter limit: the loop leaves through its guard when `r5 == limit`.
    pub limit: u32,
}

/// `body` memory uops with post-indexed pointers, `r5 += 1`, `cmp r5, #limit`, a guard that
/// leaves when equal (fused into one uop when `fused`) and the `LOOP_BACK`. Exit 0 is the
/// `LoopBack` exit (its `done` is the sum of `Uop::n` of one iteration), exit 1 the guard's,
/// exit 2 the stale exit of the stores.
pub(crate) fn loop_trace(rng: &mut Rng, fused: bool) -> LoopTrace {
    let mut uops: Vec<Uop> = Vec::new();
    let mut pc = 0x1000u32;
    let wb_post = F_U | F_WB;
    let limit = 1 + rng.below(5);
    for _ in 0..1 + rng.below(3) {
        let mut u = match rng.below(5) {
            0 => {
                let mut u = Uop::new(k::LDR_IMM, pc);
                (u.rd, u.rn, u.rs, u.imm) = (8, 0, wb_post, 4);
                u
            }
            1 => {
                let mut u = Uop::new(k::STR_IMM, pc);
                (u.rd, u.rn, u.rs, u.imm) = (9, 1, wb_post, 4);
                u
            }
            2 => {
                let mut u = Uop::new(k::LDM, pc);
                (u.rn, u.rs, u.imm, u.sh) = (2, F_U | F_WB, 0b1100_0000_0000, 2);
                u
            }
            3 => {
                let mut u = Uop::new(k::STM, pc);
                (u.rn, u.rs, u.imm, u.sh) = (3, F_U | F_P | F_WB, 0b0011_0000_0000_0000, 2);
                u
            }
            _ => {
                let mut u = Uop::new(k::LDRH, pc);
                (u.rd, u.rn, u.rs, u.imm) = (8, 0, wb_post, 2);
                u
            }
        };
        u.set_exit_idx(2);
        uops.push(u);
        pc += 4;
    }
    uops.push(add_imm(5, 5, 1, pc));
    pc += 4;
    let mut cmp = Uop::new(k::CMP_IMM, pc);
    (cmp.rn, cmp.imm) = (5, limit);
    let mut guard = Uop::new(k::GUARD, pc + 4);
    (guard.cond, guard.rs) = (0, 1);
    guard.n = 1;
    let n_iter = uops.iter().map(|u| u16::from(u.n)).sum::<u16>() + 2;
    match fuse_pair(&cmp, &guard).filter(|_| fused) {
        Some(f) => uops.push(f),
        None => {
            uops.push(cmp);
            uops.push(guard);
        }
    }
    let mut back = Uop::new(k::LOOP_BACK, pc + 8);
    (back.rs, back.n) = (0, 0);
    uops.push(back);
    let exits = vec![
        Exit::new(ExitKind::LoopBack, n_iter, pc + 4, 0x1000),
        Exit::new(ExitKind::Direct, n_iter, pc + 4, 0x2000),
        Exit::new(ExitKind::Stale, 1, 0x1000, 0x1004),
    ];
    let sb = Sb::from_parts(0x1000, n_iter, uops, exits, vec![(0x1000, 0)]).expect("small");
    LoopTrace { sb, limit }
}

/// A case for a loop trace: pointers near the end of a page (so a few iterations run on fast
/// pages and then reach a slow one), the counter at 0..=2, a budget of 0 to 6 more laps.
pub(crate) fn loop_case(rng: &mut Rng, t: &LoopTrace, watch: bool) -> Case {
    let mut cpu = random_state(rng, true);
    for reg in cpu.r.iter_mut().take(4) {
        let page = [RAM0, RAM1, RAM2, HOOKED_R][rng.below(4) as usize];
        *reg = page_addr(page) + 0x1000 - 4 * rng.range(1, 9);
    }
    cpu.r[5] = rng.below(3);
    let n = u64::from(t.sb.n_guest);
    let mut case = Case::new(cpu, rng.below(4));
    case.left = n + rng.below(7) as u64 * n / 2;
    case.watch = watch;
    case
}

/// Loops against tier 2: every budget, the watch exit, fused and unfused guards.
pub(crate) fn loop_suite(
    jit: &mut Jit,
    rng: &mut Rng,
    traces: u32,
    cases: u32,
) -> Result<Tally, String> {
    let mut t = Tally::default();
    for n in 0..traces {
        let lt = loop_trace(rng, n % 2 == 0);
        let native = compile_test(jit, &lt.sb).ok_or("the loop did not compile")?;
        for c in 0..cases {
            let case = loop_case(rng, &lt, c % 5 == 4);
            run_case(jit, &native, &lt.sb, &case, &mut t)
                .map_err(|e| format!("loop {n} (limit {}, fused {}): {e}", lt.limit, n % 2 == 0))?;
        }
    }
    Ok(t)
}

/// The flow half of the kill suite: loops, plus a few traces of the condition codes.
pub(crate) fn kill_suite(jit: &mut Jit) -> Result<Tally, String> {
    let mut rng = Rng::new(0xF10E);
    let mut t = loop_suite(jit, &mut rng, 10, 12)?;
    t.merge(&cond_suite(jit, &mut rng)?);
    t.merge(&pair_suite(jit, &mut rng, 6, 10)?);
    t.merge(&guard_eq_suite(jit, &mut rng)?);
    Ok(t)
}

/// `GUARD_REG_EQ` against registers equal to the value, equal in the low halfword only, equal in
/// the top bits only, and random.
pub(crate) fn guard_eq_suite(jit: &mut Jit, rng: &mut Rng) -> Result<Tally, String> {
    let mut t = Tally::default();
    for imm in [0x9000_0010u32, 0xFFFF_FFF0, 0x10] {
        let mut g = Uop::new(k::GUARD_REG_EQ, 0x1000);
        (g.rn, g.imm, g.rs, g.n) = (3, imm, 0, 0);
        let exits = vec![
            Exit::new(ExitKind::Direct, 1, 0x1000, 0x2000),
            Exit::new(ExitKind::Direct, 2, 0x1004, 0x1008),
        ];
        let sb = Sb::from_parts(
            0x1000,
            2,
            vec![g, Uop::exit(1, 0x1004)],
            exits,
            vec![(0x1000, 0)],
        )
        .unwrap();
        let native = compile_test(jit, &sb).ok_or("did not compile")?;
        for value in [
            imm,
            imm ^ 0xA000_0000,
            imm ^ 1,
            imm ^ 0x1_0000,
            imm & 0xFFFF,
            rng.next(),
        ] {
            let mut cpu = random_state(rng, true);
            cpu.r[3] = value;
            run_case(jit, &native, &sb, &Case::new(cpu, 0), &mut t)
                .map_err(|e| format!("guard_reg_eq {imm:#x} vs {value:#x}: {e}"))?;
        }
    }
    Ok(t)
}

/// One `GUARD cond` trace for every condition, run on all 16 flag nibbles.
pub(crate) fn cond_suite(jit: &mut Jit, rng: &mut Rng) -> Result<Tally, String> {
    let mut t = Tally::default();
    for cond in 0..16u8 {
        let mut g = Uop::new(k::GUARD, 0x1000);
        (g.cond, g.rs, g.n) = (cond, 0, 0);
        let exits = vec![
            Exit::new(ExitKind::Direct, 1, 0x1000, 0x2000),
            Exit::new(ExitKind::Direct, 2, 0x1004, 0x1008),
        ];
        let uops = vec![g, Uop::exit(1, 0x1004)];
        let sb = Sb::from_parts(0x1000, 2, uops, exits, vec![(0x1000, 0)]).unwrap();
        let native = compile_test(jit, &sb).ok_or("the guard did not compile")?;
        for nzcv in 0..16u32 {
            let mut cpu = random_state(rng, true);
            (cpu.n, cpu.z, cpu.c, cpu.v) =
                (nzcv & 8 != 0, nzcv & 4 != 0, nzcv & 2 != 0, nzcv & 1 != 0);
            run_case(jit, &native, &sb, &Case::new(cpu, 0), &mut t)
                .map_err(|e| format!("GUARD cond {cond} nzcv {nzcv:#06b}: {e}"))?;
        }
    }
    Ok(t)
}

/// `MOV_PAIR`, `LDR_PAIR` and the compare-and-guard pairs on random states (the second load
/// of a pair on a slow page included).
pub(crate) fn pair_suite(
    jit: &mut Jit,
    rng: &mut Rng,
    traces: u32,
    cases: u32,
) -> Result<Tally, String> {
    let mut t = Tally::default();
    for n in 0..traces {
        let mut a = Uop::new(k::LDR_IMM, 0x1000);
        (a.rd, a.rn, a.rs, a.imm) = (rng.below(7) as u8, 7, F_U | F_P, 0xFFC - 4 * rng.below(2));
        let mut b = Uop::new(k::LDR_IMM, 0x1004);
        (b.rd, b.rn, b.rs, b.imm) = (8 + rng.below(6) as u8, 7, F_U | F_P, 4 * rng.below(3));
        let ldr = fuse_pair(&a, &b).expect("a loadable pair");
        let mut m1 = Uop::new(k::MOV_IMM, 0x1000);
        (m1.rd, m1.imm) = (rng.below(8) as u8, rng.below(0x1_0000));
        let mut m2 = Uop::new(k::MOV_IMM, 0x1004);
        (m2.rd, m2.imm) = (8 + rng.below(6) as u8, rng.below(0x1_0000));
        let mov = fuse_pair(&m1, &m2).expect("a movable pair");
        let mut cmp = Uop::new(if n % 2 == 0 { k::CMP_IMM } else { k::CMP_REG }, 0x1000);
        (cmp.rn, cmp.rm, cmp.imm) = (rng.below(8) as u8, 8 + rng.below(6) as u8, rng.below(8));
        if cmp.kind == k::CMP_REG {
            cmp.imm = 0;
        } else {
            cmp.rm = 0;
        }
        let mut g = Uop::new(k::GUARD, 0x1004);
        (g.cond, g.rs, g.n) = (rng.below(14) as u8, 1, 1);
        let guard = fuse_pair(&cmp, &g).expect("a compare-guard pair");
        let exits = vec![
            Exit::new(ExitKind::Stale, 1, 0x1000, 0x1004),
            Exit::new(ExitKind::Direct, 3, 0x1004, 0x3000),
            Exit::new(ExitKind::Direct, 6, 0x1010, 0x1014),
        ];
        let uops = vec![
            ldr,
            mov,
            guard,
            add_imm(2, 2, 1, 0x1010),
            Uop::exit(2, 0x1010),
        ];
        let sb = Sb::from_parts(0x1000, 6, uops, exits, vec![(0x1000, 0)]).unwrap();
        let native = compile_test(jit, &sb).ok_or("the pairs did not compile")?;
        for c in 0..cases {
            let mut cpu = random_state(rng, true);
            cpu.r[7] =
                page_addr([RAM0, RAM1, HOOKED_R][(c % 3) as usize]) + 0x1000 - 4 * rng.range(1, 6);
            run_case(jit, &native, &sb, &Case::new(cpu, c % 3), &mut t)
                .map_err(|e| format!("pairs {n}: {e}"))?;
        }
    }
    Ok(t)
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn a_loop_trace_ends_in_its_back_edge_and_has_the_three_exits() {
        let mut rng = Rng::new(9);
        for fused in [false, true] {
            let lt = loop_trace(&mut rng, fused);
            let last = lt.sb.uops.last().expect("uops");
            assert_eq!(last.kind, k::LOOP_BACK);
            assert_eq!(lt.sb.exits.len(), 3);
            assert_eq!(lt.sb.exits[0].kind, ExitKind::LoopBack);
            assert_eq!(
                lt.sb.uops.iter().any(|u| u.kind == k::CMP_GUARD_I),
                fused,
                "the compare and the guard are one uop exactly when fused"
            );
            let case = loop_case(&mut rng, &lt, true);
            assert!(case.watch && case.left >= u64::from(lt.sb.n_guest));
        }
    }
}
